//! Debugging for PeroxIDE over the Debug Adapter Protocol ([ADR 0012]).
//!
//! [ADR 0012]: https://github.com/deveshrai/perox-tools/blob/main/docs/adr/0012-debugger.md

pub mod adapter;
pub mod client;
pub mod session;

use std::path::Path;
use std::sync::mpsc::Receiver;

pub use client::{Client, Message};
pub use session::{LaunchConfig, Session, State};

/// A live session: the adapter process plus the session model.
#[derive(Debug)]
pub struct Debugger {
    client: Client,
    messages: Receiver<Message>,
    pub session: Session,
}

impl Debugger {
    /// Starts `adapter` and begins launching the program. `wake` runs whenever the
    /// adapter says something.
    pub fn start(
        adapter: &Path,
        config: LaunchConfig,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let (client, messages) = Client::spawn(adapter, &[], &config.cwd, wake)?;
        let mut d = Self {
            client,
            messages,
            session: Session::new(config),
        };
        d.flush();
        Ok(d)
    }

    /// Handles what has arrived and sends what that calls for. True if anything came.
    pub fn poll(&mut self) -> bool {
        let mut any = false;
        while let Ok(m) = self.messages.try_recv() {
            self.session.handle(m);
            any = true;
        }
        self.flush();
        any
    }

    /// Sends queued requests (call after using the session's methods).
    pub fn flush(&mut self) {
        for out in self.session.take_outbox() {
            let sent = match &out {
                session::Outgoing::Request {
                    seq,
                    command,
                    arguments,
                } => self.client.send_request(*seq, command, arguments),
                session::Outgoing::Response {
                    seq,
                    request_seq,
                    command,
                    success,
                } => self.client.respond(
                    *seq,
                    *request_seq,
                    command,
                    *success,
                    serde_json::Value::Null,
                ),
            };
            if sent.is_err() {
                self.session.handle(Message::Exited);
            }
        }
    }

    /// Ends the program and the adapter.
    pub fn kill(&mut self) {
        self.client.kill();
        self.session.handle(Message::Exited);
    }
}
