//! The Streamable HTTP endpoint (`POST /mcp`, JSON responses, no event stream).

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use perox_core::Access;
use serde_json::{Value, json};

use crate::http::{self, ReadError, Request, Response};
use crate::tools::{self, Host};

/// MCP protocol revisions this server speaks, newest first.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const MAX_SESSIONS: usize = 16;
const MAX_CONNECTIONS: usize = 32;

/// How to run the server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// `127.0.0.1` for this computer only, `0.0.0.0` for the network.
    pub bind: IpAddr,
    /// TCP port; 0 picks a free one (tests).
    pub port: u16,
    /// Every request must carry `Authorization: Bearer <token>`.
    pub token: String,
    /// The highest access level agents get.
    pub max_access: Access,
}

/// What the status line shows.
#[derive(Debug, Clone, Default)]
pub struct Stats {
    /// Open MCP sessions.
    pub sessions: usize,
    /// Authorised requests handled.
    pub requests: u64,
    /// When the last authorised request arrived.
    pub last_request: Option<Instant>,
    /// Requests refused for a missing or wrong token.
    pub refused: u64,
}

struct Shared {
    config: ServerConfig,
    host: Arc<dyn Host>,
    sessions: Mutex<HashMap<String, Instant>>,
    stats: Mutex<Stats>,
    stop: AtomicBool,
    connections: AtomicUsize,
}

/// A running server. Dropping it stops it.
pub struct Server {
    shared: Arc<Shared>,
    addr: SocketAddr,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").field("addr", &self.addr).finish()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Server {
    /// Binds and starts serving on a background thread.
    pub fn start(config: ServerConfig, host: Arc<dyn Host>) -> io::Result<Self> {
        let listener = TcpListener::bind((config.bind, config.port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            config,
            host,
            sessions: Mutex::new(HashMap::new()),
            stats: Mutex::new(Stats::default()),
            stop: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
        });
        let s = shared.clone();
        let thread = thread::Builder::new()
            .name("mcp-accept".into())
            .spawn(move || accept_loop(&listener, &s))?;
        Ok(Self {
            shared,
            addr,
            thread: Some(thread),
        })
    }

    /// The address actually bound (useful with port 0).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// A snapshot for the status line.
    pub fn stats(&self) -> Stats {
        let mut stats = lock(&self.shared.stats).clone();
        stats.sessions = lock(&self.shared.sessions).len();
        stats
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    while !shared.stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if shared.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    shared.connections.fetch_sub(1, Ordering::SeqCst);
                    let mut stream = stream;
                    let _ = stream.set_nonblocking(false);
                    let _ = Response::text(503, "too many connections").write_to(&mut stream);
                    continue;
                }
                let s = shared.clone();
                let spawned = thread::Builder::new()
                    .name("mcp-conn".into())
                    .spawn(move || {
                        handle_connection(stream, &s);
                        s.connections.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    shared.connections.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn handle_connection(mut stream: TcpStream, shared: &Shared) {
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let response = match http::read_request(&mut stream) {
        Ok(request) => handle(&request, shared),
        Err(ReadError::Closed) => return,
        Err(ReadError::Bad(why)) => Response::text(400, why),
        Err(ReadError::TooLarge) => Response::text(413, "request too large"),
        Err(ReadError::LengthRequired) => Response::text(411, "Content-Length required"),
    };
    let _ = response.write_to(&mut stream);
}

fn handle(req: &Request, shared: &Shared) -> Response {
    if req.path != "/mcp" {
        return Response::text(404, "the MCP endpoint is /mcp");
    }
    // DNS-rebinding protection: browsers send Origin; MCP clients normally don't.
    if let Some(origin) = req.header("origin")
        && !is_local_origin(origin)
    {
        return Response::text(403, "origin not allowed");
    }
    if !authorised(req, &shared.config.token) {
        lock(&shared.stats).refused += 1;
        return Response::text(401, "missing or wrong token")
            .with_header("WWW-Authenticate", "Bearer".into());
    }
    {
        let mut stats = lock(&shared.stats);
        stats.requests += 1;
        stats.last_request = Some(Instant::now());
    }
    match req.method.as_str() {
        "POST" => post(req, shared),
        "DELETE" => match req.header("mcp-session-id") {
            Some(id) if lock(&shared.sessions).remove(id).is_some() => Response::empty(200),
            _ => Response::text(404, "unknown session"),
        },
        _ => Response::text(405, "use POST").with_header("Allow", "POST, DELETE".into()),
    }
}

fn post(req: &Request, shared: &Shared) -> Response {
    let Ok(msg) = serde_json::from_slice::<Value>(&req.body) else {
        return Response::json(&rpc_error(Value::Null, -32700, "parse error")).status(400);
    };
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        // A response or something else from the client: nothing to answer.
        return Response::empty(202);
    };
    let Some(id) = msg.get("id").cloned() else {
        // A notification (e.g. notifications/initialized).
        return Response::empty(202);
    };
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    if method == "initialize" {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let version = PROTOCOL_VERSIONS
            .iter()
            .find(|v| **v == requested)
            .unwrap_or(&PROTOCOL_VERSIONS[0]);
        let session = crate::generate_token()[..32].to_owned();
        {
            let mut sessions = lock(&shared.sessions);
            if sessions.len() >= MAX_SESSIONS
                && let Some(oldest) = sessions
                    .iter()
                    .min_by_key(|(_, t)| **t)
                    .map(|(k, _)| k.clone())
            {
                sessions.remove(&oldest);
            }
            sessions.insert(session.clone(), Instant::now());
        }
        let result = json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "peroxide", "title": "PeroxIDE", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "This server controls a running PeroxIDE, a Rust IDE. Everything \
                the user can do from menus is a command. Call list_commands to see them, \
                describe_command for arguments, and execute or batch to run them. The user \
                watches your actions in the IDE and can undo edits."
        });
        return Response::json(&rpc_result(id, result)).with_header("Mcp-Session-Id", session);
    }

    match req.header("mcp-session-id") {
        None => {
            return Response::json(&rpc_error(id, -32600, "missing Mcp-Session-Id")).status(400);
        }
        Some(s) => match lock(&shared.sessions).get_mut(s) {
            Some(seen) => *seen = Instant::now(),
            None => return Response::text(404, "unknown session; initialize again"),
        },
    }

    let reply = match method {
        "ping" => rpc_result(id, json!({})),
        "tools/list" => rpc_result(id, tools::list()),
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match tools::call(shared.host.as_ref(), name, &args, shared.config.max_access) {
                Ok(result) => rpc_result(id, result),
                Err(tools::UnknownTool(n)) => rpc_error(id, -32602, &format!("unknown tool `{n}`")),
            }
        }
        _ => rpc_error(id, -32601, &format!("method `{method}` not supported")),
    };
    Response::json(&reply)
}

impl Response {
    fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn authorised(req: &Request, token: &str) -> bool {
    let Some(given) = req
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    constant_time_eq(given.trim().as_bytes(), token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_local_origin(origin: &str) -> bool {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or("");
    let host = if rest.starts_with('[') {
        rest.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        rest.split([':', '/']).next().unwrap_or_default().to_owned()
    };
    matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert!(is_local_origin("http://localhost:3000"));
        assert!(is_local_origin("http://127.0.0.1"));
        assert!(is_local_origin("http://[::1]:8080"));
        assert!(!is_local_origin("http://evil.example"));
        assert!(!is_local_origin("http://localhost.evil.example"));
        assert!(!is_local_origin("null"));
    }

    #[test]
    fn constant_time() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
