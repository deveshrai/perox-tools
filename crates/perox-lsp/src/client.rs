//! A JSON-RPC client over a child process's stdin/stdout (the LSP base protocol).
//! Never blocks the caller: responses and notifications arrive on a channel, and an
//! optional `wake` callback runs on arrival (the app uses it to repaint).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

/// Something the server sent.
#[derive(Debug, Clone)]
pub enum Event {
    Response {
        id: i64,
        result: Result<Value, String>,
    },
    Notification {
        method: String,
        params: Value,
    },
    /// A request we don't answer automatically; reply with [`Client::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The server's stderr, line by line (for the log).
    Stderr(String),
    Exited,
}

type Waiters = Arc<Mutex<HashMap<i64, Sender<Result<Value, String>>>>>;

pub struct Client {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    /// Requests whose answer someone is waiting for synchronously (startup only).
    waiters: Waiters,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("pid", &self.child.id())
            .finish()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Requests the server makes that every client answers the same way.
fn auto_answer(method: &str) -> Option<Value> {
    match method {
        "window/workDoneProgress/create"
        | "client/registerCapability"
        | "client/unregisterCapability" => Some(Value::Null),
        "workspace/configuration" => Some(json!([])),
        _ => None,
    }
}

impl Client {
    /// Starts `program` in `cwd`. Events go to the returned receiver.
    pub fn spawn(
        program: &Path,
        args: &[&str],
        cwd: &Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<(Self, Receiver<Event>)> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().expect("piped")));
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let (tx, rx) = mpsc::channel();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let waiters: Waiters = Arc::default();

        {
            let (tx, wake) = (tx.clone(), wake.clone());
            thread::Builder::new()
                .name("lsp-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let _ = tx.send(Event::Stderr(line));
                    }
                    wake();
                })?;
        }
        {
            let (stdin, waiters) = (stdin.clone(), waiters.clone());
            thread::Builder::new()
                .name("lsp-read".into())
                .spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    while let Some(msg) = read_message(&mut reader) {
                        let event = classify(msg);
                        let event = match event {
                            Event::Request { id, method, params } => match auto_answer(&method) {
                                Some(result) => {
                                    let _ = write_message(
                                        &mut *lock(&stdin),
                                        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
                                    );
                                    continue;
                                }
                                None => Event::Request { id, method, params },
                            },
                            Event::Response { id, result } => match lock(&waiters).remove(&id) {
                                Some(w) => {
                                    let _ = w.send(result);
                                    continue;
                                }
                                None => Event::Response { id, result },
                            },
                            other => other,
                        };
                        if tx.send(event).is_err() {
                            break;
                        }
                        wake();
                    }
                    let _ = tx.send(Event::Exited);
                    wake();
                })?;
        }
        Ok((
            Self {
                child,
                stdin,
                next_id: AtomicI64::new(1),
                waiters,
            },
            rx,
        ))
    }

    /// Sends a request; its response arrives as [`Event::Response`] with this id.
    pub fn request(&self, method: &str, params: Value) -> i64 {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    /// Sends a request and waits for the answer. For startup only: never on the UI thread.
    pub fn request_blocking(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        lock(&self.waiters).insert(id, tx);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        rx.recv_timeout(timeout)
            .map_err(|_| format!("{method}: no answer"))?
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    pub fn respond(&self, id: Value, result: Value) {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    pub fn cancel(&self, id: i64) {
        self.notify("$/cancelRequest", json!({ "id": id }));
    }

    fn send(&self, msg: &Value) {
        let _ = write_message(&mut *lock(&self.stdin), msg);
    }

    /// Asks the server to shut down, then makes sure the process is gone.
    pub fn shutdown(mut self) {
        let _ = self.request_blocking("shutdown", Value::Null, Duration::from_secs(2));
        self.notify("exit", Value::Null);
        thread::sleep(Duration::from_millis(100));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn classify(msg: Value) -> Event {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    match (msg.get("id").cloned(), method) {
        (Some(id), Some(method)) => Event::Request { id, method, params },
        (None, Some(method)) => Event::Notification { method, params },
        (Some(id), None) => {
            let id = id.as_i64().unwrap_or(-1);
            let result = match msg.get("error") {
                Some(e) => Err(e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_owned()),
                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
            };
            Event::Response { id, result }
        }
        (None, None) => Event::Notification {
            method: String::new(),
            params,
        },
    }
}

pub fn write_message(w: &mut impl Write, msg: &Value) -> std::io::Result<()> {
    let body = msg.to_string();
    write!(w, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    w.flush()
}

pub fn read_message(r: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            length = v.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    r.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_roundtrip() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"a": "é"})).unwrap();
        write_message(&mut buf, &json!({"b": 2})).unwrap();
        let mut r = std::io::BufReader::new(&buf[..]);
        assert_eq!(read_message(&mut r), Some(json!({"a": "é"})));
        assert_eq!(read_message(&mut r), Some(json!({"b": 2})));
        assert_eq!(read_message(&mut r), None);
    }

    #[test]
    fn classifies_messages() {
        assert!(matches!(
            classify(json!({"id": 3, "result": 1})),
            Event::Response {
                id: 3,
                result: Ok(_)
            }
        ));
        assert!(matches!(
            classify(json!({"id": 3, "error": {"message": "x"}})),
            Event::Response { result: Err(_), .. }
        ));
        assert!(matches!(
            classify(json!({"method": "m"})),
            Event::Notification { .. }
        ));
        assert!(matches!(
            classify(json!({"id": "a", "method": "m"})),
            Event::Request { .. }
        ));
    }
}
