//! Starts a real server on a free port with a fake IDE behind it and talks raw HTTP to it.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};

use perox_core::{Access, CommandBus, CommandError, CommandSpec, Origin};
use perox_mcp::{Host, Server, ServerConfig};
use serde_json::{Value, json};

struct FakeIde {
    bus: CommandBus<i64>,
    state: Mutex<i64>,
}

impl Host for FakeIde {
    fn commands(&self, max_access: Access) -> Vec<CommandSpec> {
        self.bus.agent_specs(max_access).cloned().collect()
    }
    fn execute(&self, id: &str, args: Value, origin: Origin) -> Result<Value, CommandError> {
        self.bus
            .execute(&mut self.state.lock().unwrap(), id, args, origin)
    }
}

const TOKEN: &str = "test-token-0123456789";

fn start(max_access: Access) -> Server {
    let mut bus = CommandBus::new();
    bus.register(
        CommandSpec::new("app.info", "Info", Access::Read),
        |_, _| Ok(json!({ "name": "PeroxIDE" })),
    )
    .unwrap();
    bus.register(
        CommandSpec::new("counter.add", "Add", Access::Edit),
        |n: &mut i64, inv| {
            *n += inv.args.get("by").and_then(Value::as_i64).unwrap_or(1);
            Ok(json!(*n))
        },
    )
    .unwrap();
    bus.register(
        CommandSpec::new("app.quit", "Quit", Access::Execute).ui_only(),
        |_, _| Ok(Value::Null),
    )
    .unwrap();
    let host = Arc::new(FakeIde {
        bus,
        state: Mutex::new(0),
    });
    Server::start(
        ServerConfig {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: 0,
            token: TOKEN.into(),
            max_access,
        },
        host,
    )
    .unwrap()
}

struct Reply {
    status: u16,
    headers: String,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<String> {
        self.headers.lines().find_map(|l| {
            let (n, v) = l.split_once(':')?;
            n.eq_ignore_ascii_case(name).then(|| v.trim().to_owned())
        })
    }
}

fn send(addr: SocketAddr, method: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    let mut s = TcpStream::connect(addr).unwrap();
    let mut req = format!(
        "{method} /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (n, v) in headers {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Reply {
        status,
        headers: head.to_owned(),
        body: body.to_owned(),
    }
}

const AUTH: (&str, &str) = ("Authorization", "Bearer test-token-0123456789");

fn initialize(addr: SocketAddr) -> String {
    let r = send(
        addr,
        "POST",
        &[AUTH],
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r.json()["result"]["serverInfo"]["name"], "peroxide");
    r.header("mcp-session-id").expect("session header")
}

fn call(addr: SocketAddr, session: &str, tool: &str, args: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": tool, "arguments": args } });
    let r = send(
        addr,
        "POST",
        &[AUTH, ("Mcp-Session-Id", session)],
        &body.to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    r.json()
}

#[test]
fn refuses_without_the_right_token() {
    let server = start(Access::Execute);
    let addr = server.local_addr();
    assert_eq!(send(addr, "POST", &[], "{}").status, 401);
    assert_eq!(
        send(addr, "POST", &[("Authorization", "Bearer wrong")], "{}").status,
        401
    );
    assert_eq!(server.stats().refused, 2);
    assert_eq!(server.stats().requests, 0);
}

#[test]
fn refuses_browser_origins() {
    let server = start(Access::Execute);
    let r = send(
        server.local_addr(),
        "POST",
        &[AUTH, ("Origin", "http://evil.example")],
        "{}",
    );
    assert_eq!(r.status, 403);
}

#[test]
fn full_session_lifecycle() {
    let server = start(Access::Execute);
    let addr = server.local_addr();
    let session = initialize(addr);

    let r = send(
        addr,
        "POST",
        &[AUTH, ("Mcp-Session-Id", &session)],
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    assert_eq!(r.status, 202);

    let r = send(
        addr,
        "POST",
        &[AUTH, ("Mcp-Session-Id", &session)],
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    );
    let names: Vec<_> = r.json()["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        names,
        ["list_commands", "describe_command", "execute", "batch"]
    );

    let r = call(addr, &session, "execute", json!({ "command": "app.info" }));
    assert_eq!(r["result"]["isError"], false);
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("PeroxIDE")
    );

    let r = call(addr, &session, "list_commands", json!({}));
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("counter.add") && !text.contains("app.quit"),
        "{text}"
    );

    let r = call(
        addr,
        &session,
        "batch",
        json!({ "steps": [
            { "command": "counter.add", "args": { "by": 2 } },
            { "command": "counter.add", "args": { "by": 3 } }
        ]}),
    );
    assert_eq!(r["result"]["isError"], false);
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(r#""result":5"#)
    );

    // UI-only commands look like unknown ones.
    let r = call(addr, &session, "execute", json!({ "command": "app.quit" }));
    assert_eq!(r["result"]["isError"], true);

    let r = call(addr, &session, "nope", json!({}));
    assert_eq!(r["error"]["code"], -32602);

    assert_eq!(server.stats().sessions, 1);
    assert_eq!(
        send(addr, "DELETE", &[AUTH, ("Mcp-Session-Id", &session)], "").status,
        200
    );
    assert_eq!(server.stats().sessions, 0);
    let r = send(
        addr,
        "POST",
        &[AUTH, ("Mcp-Session-Id", &session)],
        r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#,
    );
    assert_eq!(
        r.status, 404,
        "a deleted session must tell the client to re-initialize"
    );
}

#[test]
fn access_level_is_enforced() {
    let server = start(Access::Read);
    let addr = server.local_addr();
    let session = initialize(addr);
    let r = call(
        addr,
        &session,
        "execute",
        json!({ "command": "counter.add" }),
    );
    assert_eq!(r["result"]["isError"], true);
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Edit")
    );
    let r = call(addr, &session, "list_commands", json!({}));
    assert!(
        !r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("counter.add")
    );
}

#[test]
fn protocol_edges() {
    let server = start(Access::Execute);
    let addr = server.local_addr();
    assert_eq!(send(addr, "GET", &[AUTH], "").status, 405);
    let r = send(addr, "POST", &[AUTH], "not json");
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"]["code"], -32700);
    let r = send(
        addr,
        "POST",
        &[AUTH],
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
    );
    assert_eq!(r.status, 400, "requests after initialize need a session");
    let session = initialize(addr);
    let r = send(
        addr,
        "POST",
        &[AUTH, ("Mcp-Session-Id", &session)],
        r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#,
    );
    assert_eq!(r.json()["error"]["code"], -32601);
}
