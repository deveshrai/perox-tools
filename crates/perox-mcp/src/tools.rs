//! The tools an agent sees. Instead of one MCP tool per IDE command (hundreds, which
//! would flood the agent's context, and MCP tool names can't contain dots), agents
//! get four tools that reach every agent-visible command on the bus:
//! `list_commands`, `describe_command`, `execute` and `batch`. SolidCraft's server
//! uses the same shape.

use perox_core::{Access, CommandError, CommandSpec, Origin};
use serde_json::{Value, json};

/// What the server needs from the IDE. The app implements it by forwarding to the
/// command bus on its main thread.
pub trait Host: Send + Sync + 'static {
    /// Agent-visible commands with `access <= max_access`, sorted by id.
    fn commands(&self, max_access: Access) -> Vec<CommandSpec>;
    /// Runs a command. May block until the IDE has run it.
    fn execute(&self, id: &str, args: Value, origin: Origin) -> Result<Value, CommandError>;
}

/// The `tools/list` result.
pub fn list() -> Value {
    json!({ "tools": [
        {
            "name": "list_commands",
            "title": "List IDE commands",
            "description": "Lists the PeroxIDE commands you can run with `execute`: id, title, \
                description and access level (read, edit, execute). Filter by id prefix, e.g. \
                \"file.\" or \"designer.\".",
            "inputSchema": { "type": "object", "properties": {
                "prefix": { "type": "string", "description": "Only ids starting with this." }
            }},
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "describe_command",
            "title": "Describe an IDE command",
            "description": "Full description of one command, including the JSON Schema of its arguments.",
            "inputSchema": { "type": "object", "required": ["command"], "properties": {
                "command": { "type": "string", "description": "Command id, e.g. \"app.info\"." }
            }},
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "execute",
            "title": "Run an IDE command",
            "description": "Runs one PeroxIDE command, exactly as if the user chose it from a menu. \
                Edits land on the IDE's undo stack. Use list_commands to find ids.",
            "inputSchema": { "type": "object", "required": ["command"], "properties": {
                "command": { "type": "string", "description": "Command id, e.g. \"app.info\"." },
                "args": { "type": "object", "description": "Arguments; see describe_command." }
            }}
        },
        {
            "name": "batch",
            "title": "Run several IDE commands",
            "description": "Runs commands in order and returns each result. Stops at the first \
                failure unless stop_on_error is false.",
            "inputSchema": { "type": "object", "required": ["steps"], "properties": {
                "steps": { "type": "array", "items": {
                    "type": "object", "required": ["command"], "properties": {
                        "command": { "type": "string" },
                        "args": { "type": "object" }
                    }
                }},
                "stop_on_error": { "type": "boolean", "default": true }
            }}
        }
    ]})
}

/// An unknown tool name: a protocol error, not a tool result.
#[derive(Debug)]
pub struct UnknownTool(pub String);

/// Runs a tool; returns an MCP `CallToolResult`.
pub fn call(
    host: &dyn Host,
    name: &str,
    args: &Value,
    max_access: Access,
) -> Result<Value, UnknownTool> {
    let origin = Origin::Agent { max_access };
    let result = match name {
        "list_commands" => {
            let prefix = args.get("prefix").and_then(Value::as_str).unwrap_or("");
            let list: Vec<Value> = host
                .commands(max_access)
                .iter()
                .filter(|s| s.id.starts_with(prefix))
                .map(|s| json!({ "id": s.id, "title": s.title, "description": s.description, "access": s.access }))
                .collect();
            Ok(json!(list))
        }
        "describe_command" => match args.get("command").and_then(Value::as_str) {
            None => Err("`command` (string) is required".to_owned()),
            Some(id) => host
                .commands(max_access)
                .into_iter()
                .find(|s| s.id == id)
                .map(|s| json!(s))
                .ok_or_else(|| CommandError::NotFound(id.to_owned()).to_string()),
        },
        "execute" => match args.get("command").and_then(Value::as_str) {
            None => Err("`command` (string) is required".to_owned()),
            Some(id) => {
                let cmd_args = args.get("args").cloned().unwrap_or(Value::Null);
                host.execute(id, cmd_args, origin)
                    .map_err(|e| e.to_string())
            }
        },
        "batch" => {
            let Some(steps) = args.get("steps").and_then(Value::as_array) else {
                return Ok(tool_result(Err("`steps` (array) is required".to_owned())));
            };
            let stop = args
                .get("stop_on_error")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut results = Vec::new();
            let mut failed = false;
            for step in steps {
                let id = step
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let cmd_args = step.get("args").cloned().unwrap_or(Value::Null);
                match host.execute(id, cmd_args, origin) {
                    Ok(v) => results.push(json!({ "command": id, "ok": true, "result": v })),
                    Err(e) => {
                        failed = true;
                        results.push(json!({ "command": id, "ok": false, "error": e.to_string() }));
                        if stop {
                            break;
                        }
                    }
                }
            }
            let body = json!({ "results": results });
            return Ok(json!({
                "content": [{ "type": "text", "text": body.to_string() }],
                "isError": failed
            }));
        }
        other => return Err(UnknownTool(other.to_owned())),
    };
    Ok(tool_result(result))
}

fn tool_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(v) => {
            let text = match v {
                Value::String(s) => s,
                Value::Null => "done".to_owned(),
                other => other.to_string(),
            };
            json!({ "content": [{ "type": "text", "text": text }], "isError": false })
        }
        Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
    }
}
