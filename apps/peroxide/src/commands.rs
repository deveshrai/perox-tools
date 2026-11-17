//! The shell's own commands. Every menu item calls one of these through the bus.

use perox_core::{Access, CommandBus, CommandSpec};
use serde_json::{Value, json};

/// State the shell's commands act on.
#[derive(Debug, Default)]
pub struct ShellState {
    pub quit_requested: bool,
    pub show_about: bool,
}

pub fn register(bus: &mut CommandBus<ShellState>) {
    let all = [
        (
            CommandSpec::new("app.info", "About This IDE", Access::Read)
                .description("Name and version of the running PeroxIDE."),
            info as fn(&mut ShellState) -> Value,
        ),
        (
            CommandSpec::new("app.quit", "Quit PeroxIDE", Access::Execute).ui_only(),
            |s| {
                s.quit_requested = true;
                Value::Null
            },
        ),
        (
            CommandSpec::new("help.about", "About PeroxIDE", Access::Read).ui_only(),
            |s| {
                s.show_about = true;
                Value::Null
            },
        ),
    ];
    for (spec, f) in all {
        bus.register(spec, move |s, _| Ok(f(s)))
            .expect("shell command ids are valid and unique");
    }
}

fn info(_: &mut ShellState) -> Value {
    json!({ "name": "PeroxIDE", "version": env!("CARGO_PKG_VERSION") })
}

#[cfg(test)]
mod tests {
    use super::*;
    use perox_core::Origin;

    #[test]
    fn shell_commands_register_and_run() {
        let mut bus = CommandBus::new();
        register(&mut bus);
        let mut s = ShellState::default();
        bus.execute(&mut s, "app.quit", Value::Null, Origin::User)
            .unwrap();
        assert!(s.quit_requested);
        let agent: Vec<_> = bus.agent_specs(Access::Execute).map(|c| c.id).collect();
        assert_eq!(agent, ["app.info"]);
    }
}
