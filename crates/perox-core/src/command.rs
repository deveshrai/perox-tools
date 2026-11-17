//! The command bus: the one entry point for every IDE action.
//!
//! Menus, keyboard shortcuts, the command palette and the MCP server all
//! invoke commands by id through [`CommandBus::execute`]. Nothing in the IDE
//! changes state any other way, so anything a user can do, an agent can do
//! (subject to [`Access`]), and everything goes through one place for undo,
//! logging and permissions. See `docs/adr/0002-command-bus.md`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a command may do. Agents are granted a maximum level; a command whose
/// level is above it is refused. Ordered: `Read < Edit < Execute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Looks at state, changes nothing (list files, read diagnostics).
    Read,
    /// Changes documents or the project (edit a file, add a widget).
    Edit,
    /// Runs programs or touches the outside world (build, run, debug, install).
    Execute,
}

/// Who invoked a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The person at the keyboard: menus, shortcuts, the command palette.
    User,
    /// An agent connected over MCP, limited to `max_access`.
    Agent {
        /// The highest [`Access`] level this agent session is allowed.
        max_access: Access,
    },
}

/// Static description of a command. Also what the MCP server publishes as a tool.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CommandSpec {
    /// Stable id, lowercase dot-separated segments: `file.save`, `designer.add_widget`.
    pub id: &'static str,
    /// Short human title for menus and the palette: "Save File".
    pub title: &'static str,
    /// One or two sentences for the palette tooltip and the agent's tool description.
    pub description: &'static str,
    /// What the command may do.
    pub access: Access,
    /// JSON Schema of the arguments object.
    pub params: Value,
    /// `false` for commands that only make sense with a person present
    /// (open a dialog, quit). Agents cannot see or run them.
    pub agent_visible: bool,
}

impl CommandSpec {
    /// A command with no arguments, visible to agents.
    pub fn new(id: &'static str, title: &'static str, access: Access) -> Self {
        Self {
            id,
            title,
            description: "",
            access,
            params: serde_json::json!({ "type": "object", "properties": {} }),
            agent_visible: true,
        }
    }

    /// Sets the description.
    pub fn description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Sets the JSON Schema of the arguments object.
    pub fn params(mut self, schema: Value) -> Self {
        self.params = schema;
        self
    }

    /// Hides the command from agents.
    pub fn ui_only(mut self) -> Self {
        self.agent_visible = false;
        self
    }
}

/// One call of a command: its arguments and who made it.
#[derive(Debug, Clone)]
pub struct Invocation {
    /// The arguments object (`Value::Null` is normalised to `{}`).
    pub args: Value,
    /// Who made the call.
    pub origin: Origin,
}

impl Invocation {
    /// A required string argument.
    pub fn str_arg(&self, name: &str) -> Result<&str, CommandError> {
        match self.args.get(name) {
            Some(Value::String(s)) => Ok(s),
            Some(_) => Err(CommandError::InvalidArgs(format!(
                "`{name}` must be a string"
            ))),
            None => Err(CommandError::InvalidArgs(format!("`{name}` is required"))),
        }
    }

    /// An optional boolean argument.
    pub fn bool_arg(&self, name: &str) -> Result<Option<bool>, CommandError> {
        match self.args.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(CommandError::InvalidArgs(format!(
                "`{name}` must be a boolean"
            ))),
        }
    }
}

/// Why a command failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    /// No such command (or it is hidden from this caller).
    #[error("unknown command `{0}`")]
    NotFound(String),
    /// The caller's access level is too low.
    #[error("command `{id}` needs {needed:?} access; this session has {granted:?}")]
    Forbidden {
        /// The command.
        id: String,
        /// The level the command needs.
        needed: Access,
        /// The level the caller has.
        granted: Access,
    },
    /// The arguments were wrong.
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    /// The command ran and failed.
    #[error("{0}")]
    Failed(String),
}

/// Why registering a command failed. Always a programming error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegisterError {
    /// The id is already taken.
    #[error("command `{0}` is already registered")]
    Duplicate(&'static str),
    /// The id does not follow the `segment.segment` rule.
    #[error(
        "invalid command id `{0}`: use two or more lowercase segments of [a-z0-9_] joined by `.`"
    )]
    InvalidId(&'static str),
}

type Handler<S> = Box<dyn Fn(&mut S, &Invocation) -> Result<Value, CommandError> + Send + Sync>;

struct Entry<S> {
    spec: CommandSpec,
    handler: Handler<S>,
}

/// The registry of commands over application state `S`.
pub struct CommandBus<S> {
    commands: BTreeMap<&'static str, Entry<S>>,
}

impl<S> fmt::Debug for CommandBus<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandBus")
            .field("commands", &self.commands.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl<S> Default for CommandBus<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> CommandBus<S> {
    /// An empty bus.
    pub fn new() -> Self {
        Self {
            commands: BTreeMap::new(),
        }
    }

    /// Adds a command. Fails on a duplicate or malformed id.
    pub fn register<F>(&mut self, spec: CommandSpec, handler: F) -> Result<(), RegisterError>
    where
        F: Fn(&mut S, &Invocation) -> Result<Value, CommandError> + Send + Sync + 'static,
    {
        if !is_valid_id(spec.id) {
            return Err(RegisterError::InvalidId(spec.id));
        }
        if self.commands.contains_key(spec.id) {
            return Err(RegisterError::Duplicate(spec.id));
        }
        self.commands.insert(
            spec.id,
            Entry {
                spec,
                handler: Box::new(handler),
            },
        );
        Ok(())
    }

    /// The spec of one command.
    pub fn spec(&self, id: &str) -> Option<&CommandSpec> {
        self.commands.get(id).map(|e| &e.spec)
    }

    /// All commands, sorted by id.
    pub fn specs(&self) -> impl Iterator<Item = &CommandSpec> {
        self.commands.values().map(|e| &e.spec)
    }

    /// The commands an agent with `max_access` may call, sorted by id.
    pub fn agent_specs(&self, max_access: Access) -> impl Iterator<Item = &CommandSpec> {
        self.specs()
            .filter(move |s| s.agent_visible && s.access <= max_access)
    }

    /// Runs a command.
    pub fn execute(
        &self,
        state: &mut S,
        id: &str,
        args: Value,
        origin: Origin,
    ) -> Result<Value, CommandError> {
        let entry = self
            .commands
            .get(id)
            .ok_or_else(|| CommandError::NotFound(id.to_owned()))?;
        if let Origin::Agent { max_access } = origin {
            if !entry.spec.agent_visible {
                // Indistinguishable from a missing command: agents don't learn UI-only ids.
                return Err(CommandError::NotFound(id.to_owned()));
            }
            if entry.spec.access > max_access {
                return Err(CommandError::Forbidden {
                    id: id.to_owned(),
                    needed: entry.spec.access,
                    granted: max_access,
                });
            }
        }
        let args = match args {
            Value::Null => Value::Object(Default::default()),
            Value::Object(_) => args,
            _ => {
                return Err(CommandError::InvalidArgs(
                    "arguments must be a JSON object".into(),
                ));
            }
        };
        (entry.handler)(state, &Invocation { args, origin })
    }
}

fn is_valid_id(id: &str) -> bool {
    let mut segments = 0;
    for segment in id.split('.') {
        let ok = !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            && !segment.starts_with(|c: char| c.is_ascii_digit());
        if !ok {
            return false;
        }
        segments += 1;
    }
    segments >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct State {
        text: String,
    }

    fn bus() -> CommandBus<State> {
        let mut bus = CommandBus::new();
        bus.register(
            CommandSpec::new("doc.read", "Read", Access::Read),
            |s: &mut State, _| Ok(json!(s.text)),
        )
        .unwrap();
        bus.register(
            CommandSpec::new("doc.set", "Set", Access::Edit),
            |s: &mut State, inv| {
                s.text = inv.str_arg("text")?.to_owned();
                Ok(Value::Null)
            },
        )
        .unwrap();
        bus.register(
            CommandSpec::new("app.quit", "Quit", Access::Execute).ui_only(),
            |_, _| Ok(Value::Null),
        )
        .unwrap();
        bus
    }

    const AGENT_EDIT: Origin = Origin::Agent {
        max_access: Access::Edit,
    };

    #[test]
    fn executes_and_mutates_state() {
        let bus = bus();
        let mut s = State::default();
        bus.execute(&mut s, "doc.set", json!({"text": "hi"}), Origin::User)
            .unwrap();
        assert_eq!(
            bus.execute(&mut s, "doc.read", Value::Null, Origin::User),
            Ok(json!("hi"))
        );
    }

    #[test]
    fn rejects_bad_registrations() {
        let mut bus = bus();
        let noop = |_: &mut State, _: &Invocation| Ok(Value::Null);
        assert_eq!(
            bus.register(CommandSpec::new("doc.read", "x", Access::Read), noop),
            Err(RegisterError::Duplicate("doc.read"))
        );
        for bad in [
            "save",
            "File.save",
            "file..save",
            "file.save!",
            ".file",
            "file.9x",
        ] {
            assert_eq!(
                bus.register(CommandSpec::new(bad, "x", Access::Read), noop),
                Err(RegisterError::InvalidId(bad))
            );
        }
        assert!(
            bus.register(
                CommandSpec::new("designer.add_widget2", "x", Access::Edit),
                noop
            )
            .is_ok()
        );
    }

    #[test]
    fn agents_cannot_see_or_run_ui_only_commands() {
        let bus = bus();
        let all: Vec<_> = bus.agent_specs(Access::Execute).map(|s| s.id).collect();
        assert_eq!(all, ["doc.read", "doc.set"]);
        let err = bus
            .execute(&mut State::default(), "app.quit", Value::Null, AGENT_EDIT)
            .unwrap_err();
        assert_eq!(err, CommandError::NotFound("app.quit".into()));
    }

    #[test]
    fn agents_are_limited_by_access() {
        let bus = bus();
        let read_only = Origin::Agent {
            max_access: Access::Read,
        };
        let listed: Vec<_> = bus.agent_specs(Access::Read).map(|s| s.id).collect();
        assert_eq!(listed, ["doc.read"]);
        let err = bus
            .execute(
                &mut State::default(),
                "doc.set",
                json!({"text": "x"}),
                read_only,
            )
            .unwrap_err();
        assert!(matches!(
            err,
            CommandError::Forbidden {
                needed: Access::Edit,
                granted: Access::Read,
                ..
            }
        ));
        assert!(
            bus.execute(
                &mut State::default(),
                "doc.set",
                json!({"text": "x"}),
                AGENT_EDIT
            )
            .is_ok()
        );
    }

    #[test]
    fn validates_arguments() {
        let bus = bus();
        let mut s = State::default();
        assert!(matches!(
            bus.execute(&mut s, "doc.set", json!([1]), Origin::User),
            Err(CommandError::InvalidArgs(_))
        ));
        assert!(matches!(
            bus.execute(&mut s, "doc.set", json!({}), Origin::User),
            Err(CommandError::InvalidArgs(m)) if m.contains("required")
        ));
        assert!(matches!(
            bus.execute(&mut s, "doc.set", json!({"text": 3}), Origin::User),
            Err(CommandError::InvalidArgs(m)) if m.contains("string")
        ));
        assert!(matches!(
            bus.execute(&mut s, "nope.nope", Value::Null, Origin::User),
            Err(CommandError::NotFound(_))
        ));
    }
}
