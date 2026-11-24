//! A Debug Adapter Protocol client over a child process's stdin/stdout. Messages use
//! the same `Content-Length` framing as LSP. Nothing blocks the caller: whatever the
//! adapter sends arrives on a channel, and `wake` runs on arrival.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use serde_json::{Value, json};

/// Something the adapter sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Response {
        request_seq: i64,
        command: String,
        success: bool,
        /// Why it failed, when it did.
        message: Option<String>,
        body: Value,
    },
    Event {
        event: String,
        body: Value,
    },
    /// A reverse request (`runInTerminal`, `startDebugging`).
    Request {
        seq: i64,
        command: String,
        arguments: Value,
    },
    /// A line the adapter wrote to stderr.
    Stderr(String),
    /// The adapter's process ended.
    Exited,
}

impl Message {
    /// Classifies one decoded DAP message.
    pub fn from_json(v: Value) -> Option<Self> {
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_owned();
        match v["type"].as_str()? {
            "response" => Some(Message::Response {
                request_seq: v["request_seq"].as_i64()?,
                command: s("command"),
                success: v["success"].as_bool().unwrap_or(false),
                message: v["message"]
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| v["body"]["error"]["format"].as_str().map(str::to_owned)),
                body: v["body"].clone(),
            }),
            "event" => Some(Message::Event {
                event: s("event"),
                body: v["body"].clone(),
            }),
            "request" => Some(Message::Request {
                seq: v["seq"].as_i64().unwrap_or(0),
                command: s("command"),
                arguments: v["arguments"].clone(),
            }),
            _ => None,
        }
    }
}

pub struct Client {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
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

impl Client {
    /// Starts the adapter. Its messages go to the returned receiver.
    pub fn spawn(
        program: &Path,
        args: &[String],
        cwd: &Path,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<(Self, Receiver<Message>)> {
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
        {
            let (tx, wake) = (tx.clone(), wake.clone());
            thread::Builder::new()
                .name("dap-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        let _ = tx.send(Message::Stderr(line));
                        wake();
                    }
                })?;
        }
        thread::Builder::new()
            .name("dap-read".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                while let Some(v) = read_message(&mut reader) {
                    if let Some(m) = Message::from_json(v) {
                        let _ = tx.send(m);
                        wake();
                    }
                }
                let _ = tx.send(Message::Exited);
                wake();
            })?;
        Ok((Self { child, stdin }, rx))
    }

    /// Sends a request with the given sequence number.
    pub fn send_request(&self, seq: i64, command: &str, arguments: &Value) -> std::io::Result<()> {
        let msg =
            json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        write_message(&mut *lock(&self.stdin), &msg)
    }

    /// Answers a reverse request.
    pub fn respond(
        &self,
        seq: i64,
        request_seq: i64,
        command: &str,
        success: bool,
        body: Value,
    ) -> std::io::Result<()> {
        let msg = json!({
            "seq": seq, "type": "response", "request_seq": request_seq,
            "command": command, "success": success, "body": body
        });
        write_message(&mut *lock(&self.stdin), &msg)
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.is_running() {
            self.kill();
        }
    }
}

/// Reads one `Content-Length`-framed JSON message. `None` at end of stream.
pub fn read_message(r: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut len = None;
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line
                .split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            {
                len = v.1.trim().parse::<usize>().ok();
            }
        }
        let Some(len) = len else { continue };
        let mut buf = vec![0; len];
        r.read_exact(&mut buf).ok()?;
        if let Ok(v) = serde_json::from_slice(&buf) {
            return Some(v);
        }
    }
}

pub fn write_message(w: &mut impl Write, v: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(v)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"type": "event", "event": "initialized"})).unwrap();
        write_message(
            &mut buf,
            &json!({"type": "response", "request_seq": 3, "command": "next", "success": true}),
        )
        .unwrap();
        let mut r = std::io::Cursor::new(buf);
        let a = Message::from_json(read_message(&mut r).unwrap()).unwrap();
        assert_eq!(
            a,
            Message::Event {
                event: "initialized".into(),
                body: Value::Null
            }
        );
        let b = Message::from_json(read_message(&mut r).unwrap()).unwrap();
        assert!(matches!(
            b,
            Message::Response {
                request_seq: 3,
                success: true,
                ..
            }
        ));
        assert!(read_message(&mut r).is_none());
    }

    #[test]
    fn failed_responses_carry_their_reason() {
        let v = json!({"type": "response", "request_seq": 1, "command": "evaluate", "success": false,
                       "body": {"error": {"format": "no such variable"}}});
        let Some(Message::Response {
            message, success, ..
        }) = Message::from_json(v)
        else {
            panic!()
        };
        assert!(!success);
        assert_eq!(message.as_deref(), Some("no such variable"));
    }
}
