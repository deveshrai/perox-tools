//! Runs the MCP server for the app and bridges its calls to the command bus.
//!
//! The server answers on its own threads; the bus and IDE state live on the UI
//! thread. A call travels as a [`Call`] over a channel, the UI thread runs it at the
//! start of the next frame ([`crate::app`]), and the reply comes back on another
//! channel.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui;
use perox_core::{Access, CommandError, CommandSpec, Origin};
use perox_mcp::{Host, Server, ServerConfig, Stats};
use serde_json::Value;

use crate::settings::{ConnectFrom, McpSettings};

/// How long an agent's call may wait for the IDE before it is told so.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// One agent call waiting for the UI thread.
#[derive(Debug)]
pub struct Call {
    pub id: String,
    pub args: Value,
    pub origin: Origin,
    pub reply: Sender<Result<Value, CommandError>>,
}

struct Bridge {
    specs: Vec<CommandSpec>,
    calls: Sender<Call>,
    ctx: egui::Context,
}

impl Host for Bridge {
    fn commands(&self, max_access: Access) -> Vec<CommandSpec> {
        self.specs
            .iter()
            .filter(|s| s.agent_visible && s.access <= max_access)
            .cloned()
            .collect()
    }

    fn execute(&self, id: &str, args: Value, origin: Origin) -> Result<Value, CommandError> {
        let (reply, answer) = mpsc::channel();
        let call = Call {
            id: id.to_owned(),
            args,
            origin,
            reply,
        };
        self.calls
            .send(call)
            .map_err(|_| CommandError::Failed("PeroxIDE is shutting down".into()))?;
        self.ctx.request_repaint();
        answer.recv_timeout(CALL_TIMEOUT).map_err(|_| {
            CommandError::Failed(
                "PeroxIDE did not answer within 2 minutes: it may be busy or showing a dialog. \
                 Ask the user to check the IDE, then try again."
                    .into(),
            )
        })?
    }
}

/// Starts and stops the server; owns what the settings page shows.
pub struct McpController {
    bridge: Arc<Bridge>,
    server: Option<Server>,
    /// Why the last start failed, in words for the settings page.
    pub error: Option<String>,
}

impl std::fmt::Debug for McpController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpController")
            .field("running", &self.server.is_some())
            .finish()
    }
}

impl McpController {
    /// `specs` is every command on the bus; the bridge filters per session.
    pub fn new(specs: Vec<CommandSpec>, ctx: egui::Context) -> (Self, Receiver<Call>) {
        let (calls, rx) = mpsc::channel();
        let bridge = Arc::new(Bridge { specs, calls, ctx });
        (
            Self {
                bridge,
                server: None,
                error: None,
            },
            rx,
        )
    }

    pub fn is_on(&self) -> bool {
        self.server.is_some()
    }

    pub fn start(&mut self, s: &McpSettings) {
        self.stop();
        let bind = match s.connect_from {
            ConnectFrom::ThisComputer => IpAddr::V4(Ipv4Addr::LOCALHOST),
            ConnectFrom::AnotherDevice => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        };
        let config = ServerConfig {
            bind,
            port: s.port,
            token: s.token.clone(),
            max_access: s.max_access,
        };
        match Server::start(config, self.bridge.clone()) {
            Ok(server) => {
                self.server = Some(server);
                self.error = None;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                self.error = Some(format!(
                    "Port {} is in use by another app. Choose another under Advanced › Port.",
                    s.port
                ));
            }
            Err(e) => self.error = Some(format!("Couldn't start the server: {e}")),
        }
    }

    pub fn stop(&mut self) {
        self.server = None;
    }

    /// Applies changed settings to a running server.
    pub fn restart_if_on(&mut self, s: &McpSettings) {
        if self.is_on() {
            self.start(s);
        }
    }

    pub fn stats(&self) -> Option<Stats> {
        self.server.as_ref().map(Server::stats)
    }

    /// The URL clients use. For "another device" it names this machine's network address.
    pub fn url(s: &McpSettings) -> Result<String, String> {
        let host = match s.connect_from {
            ConnectFrom::ThisComputer => "127.0.0.1".to_owned(),
            ConnectFrom::AnotherDevice => lan_ip()
                .ok_or("This computer isn't on a network. Join one, or choose This computer.")?
                .to_string(),
        };
        Ok(format!("http://{host}:{}/mcp", s.port))
    }
}

/// This machine's address on the local network. Connecting a UDP socket sends nothing;
/// it only asks the OS which interface would be used.
fn lan_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(1, 1, 1, 1), 80)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}
