//! A debug session as a state machine. Feed it what the adapter sends
//! ([`Session::handle`]); call its methods for what the user does; send what
//! [`Session::take_outbox`] returns. It does no I/O itself.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::client::Message;

/// How to start the program.
#[derive(Debug, Clone, Default)]
pub struct LaunchConfig {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    /// Debugger commands run before the program starts (Rust formatters, source maps).
    pub init_commands: Vec<String>,
    /// Stop when the program panics (a breakpoint on `rust_panic`).
    pub stop_on_panic: bool,
    /// Frames in files under here are "yours": a stop selects the first of them.
    pub project_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Talking to the adapter; the program hasn't started yet.
    Starting,
    Running,
    Stopped {
        thread: i64,
        /// `breakpoint`, `step`, `pause`, `exception`, `function breakpoint`, …
        reason: String,
        description: Option<String>,
    },
    /// The program ended (or the adapter did).
    Terminated,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub id: i64,
    pub name: String,
    pub path: Option<PathBuf>,
    /// 1-based.
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub name: String,
    pub reference: i64,
    pub expensive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub ty: Option<String>,
    /// Non-zero when it has children (fields, elements).
    pub reference: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Breakpoint {
    /// 1-based, as set.
    pub line: usize,
    /// The adapter's id, once it has answered.
    pub id: Option<i64>,
    /// The debugger found code for it.
    pub verified: bool,
    /// Where it actually landed, if not on `line`.
    pub actual_line: Option<usize>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleKind {
    /// The program's stdout.
    Stdout,
    Stderr,
    /// The debugger's own messages.
    Debugger,
    /// An expression the user typed.
    Input,
    Result,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleEntry {
    pub kind: ConsoleKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Watch {
    pub expression: String,
    /// The value at the last stop, or why it couldn't be evaluated.
    pub value: Option<Result<String, String>>,
    pub reference: i64,
}

/// Something to send to the adapter.
#[derive(Debug, Clone, PartialEq)]
pub enum Outgoing {
    Request {
        seq: i64,
        command: String,
        arguments: Value,
    },
    /// An answer to a reverse request.
    Response {
        seq: i64,
        request_seq: i64,
        command: String,
        success: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum Pending {
    Initialize,
    Launch,
    SetBreakpoints(PathBuf),
    FunctionBreakpoints,
    ConfigurationDone,
    Threads,
    StackTrace,
    Scopes(i64),
    Variables(i64),
    Evaluate(EvalFor),
    Other,
}

#[derive(Debug, Clone, PartialEq)]
enum EvalFor {
    Console,
    Watch(usize),
}

#[derive(Debug)]
pub struct Session {
    config: LaunchConfig,
    seq: i64,
    outbox: Vec<Outgoing>,
    pending: HashMap<i64, Pending>,
    /// The adapter has sent `initialized`: breakpoints can be set.
    configured: bool,
    pub state: State,
    pub capabilities: Value,
    pub threads: Vec<(i64, String)>,
    /// The stopped thread's call stack, innermost first.
    pub frames: Vec<Frame>,
    /// Index into `frames` of the frame being looked at.
    pub frame: Option<usize>,
    pub scopes: Vec<Scope>,
    /// Children by variables reference (scopes and expanded variables).
    pub variables: HashMap<i64, Vec<Variable>>,
    pub breakpoints: BTreeMap<PathBuf, Vec<Breakpoint>>,
    pub console: Vec<ConsoleEntry>,
    pub watches: Vec<Watch>,
    pub exit_code: Option<i64>,
    /// Why the session couldn't start or a request failed, for the status bar.
    pub error: Option<String>,
    /// Bumped whenever the program stops, so views know to refresh.
    pub stops: u64,
    /// The last breakpoint stop: LLDB's location id (`1.2`), the top function and line.
    last_stop: Option<(String, String, usize)>,
    /// The breakpoint location of the stop being loaded, and where the Continue that led
    /// to it started.
    stop_location: Option<(String, Option<(String, usize)>)>,
    /// Where (function, line) the last Continue started, while it's going on.
    continuing: Option<(String, usize)>,
}

impl Session {
    /// A new session; the first requests (`initialize`) are already queued.
    pub fn new(config: LaunchConfig) -> Self {
        let mut s = Self {
            config,
            seq: 0,
            outbox: Vec::new(),
            pending: HashMap::new(),
            configured: false,
            state: State::Starting,
            capabilities: Value::Null,
            threads: Vec::new(),
            frames: Vec::new(),
            frame: None,
            scopes: Vec::new(),
            variables: HashMap::new(),
            breakpoints: BTreeMap::new(),
            console: Vec::new(),
            watches: Vec::new(),
            exit_code: None,
            error: None,
            stops: 0,
            last_stop: None,
            stop_location: None,
            continuing: None,
        };
        s.request(
            Pending::Initialize,
            "initialize",
            json!({
                "clientID": "peroxide", "clientName": "PeroxIDE", "adapterID": "lldb-dap",
                "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path",
                "supportsVariableType": true, "supportsRunInTerminalRequest": false,
                "locale": "en"
            }),
        );
        s
    }

    pub fn config(&self) -> &LaunchConfig {
        &self.config
    }

    /// What to send now.
    pub fn take_outbox(&mut self) -> Vec<Outgoing> {
        std::mem::take(&mut self.outbox)
    }

    fn next_seq(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    fn request(&mut self, why: Pending, command: &str, arguments: Value) -> i64 {
        let seq = self.next_seq();
        self.pending.insert(seq, why);
        self.outbox.push(Outgoing::Request {
            seq,
            command: command.to_owned(),
            arguments,
        });
        seq
    }

    pub fn is_stopped(&self) -> bool {
        matches!(self.state, State::Stopped { .. })
    }

    pub fn is_live(&self) -> bool {
        self.state != State::Terminated
    }

    /// The frame being looked at.
    pub fn current_frame(&self) -> Option<&Frame> {
        self.frames.get(self.frame?)
    }

    // ── what the user does ──

    /// Sets the breakpoints of one file (1-based lines), replacing its old ones.
    pub fn set_breakpoints(&mut self, path: &Path, lines: &[usize]) {
        let old = self.breakpoints.remove(path).unwrap_or_default();
        let bps: Vec<Breakpoint> = lines
            .iter()
            .map(|&line| {
                old.iter()
                    .find(|b| b.line == line)
                    .cloned()
                    .unwrap_or(Breakpoint {
                        line,
                        id: None,
                        verified: false,
                        actual_line: None,
                        message: None,
                    })
            })
            .collect();
        if bps.is_empty() {
            // Still tell the adapter, so it clears them.
            self.breakpoints.insert(path.to_owned(), Vec::new());
            self.send_breakpoints(path);
            self.breakpoints.remove(path);
        } else {
            self.breakpoints.insert(path.to_owned(), bps);
            self.send_breakpoints(path);
        }
    }

    fn send_breakpoints(&mut self, path: &Path) {
        if !self.configured {
            return; // sent when the adapter is ready
        }
        let lines: Vec<Value> = self.breakpoints[path]
            .iter()
            .map(|b| json!({ "line": b.line }))
            .collect();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.request(
            Pending::SetBreakpoints(path.to_owned()),
            "setBreakpoints",
            json!({ "source": { "path": path, "name": name }, "breakpoints": lines, "sourceModified": false }),
        );
    }

    fn thread(&self) -> Option<i64> {
        match &self.state {
            State::Stopped { thread, .. } => Some(*thread),
            _ => self.threads.first().map(|t| t.0),
        }
    }

    fn resume(&mut self, command: &str) -> bool {
        let Some(thread) = self.thread().filter(|_| self.is_stopped()) else {
            return false;
        };
        let from = self.frames.first().map(|f| (f.name.clone(), f.line));
        self.request(Pending::Other, command, json!({ "threadId": thread }));
        self.running();
        self.continuing = if command == "continue" { from } else { None };
        true
    }

    pub fn continue_(&mut self) -> bool {
        self.resume("continue")
    }

    pub fn step_over(&mut self) -> bool {
        self.resume("next")
    }

    pub fn step_in(&mut self) -> bool {
        self.resume("stepIn")
    }

    pub fn step_out(&mut self) -> bool {
        self.resume("stepOut")
    }

    pub fn pause(&mut self) -> bool {
        if self.state != State::Running {
            return false;
        }
        let thread = self.thread().unwrap_or(0);
        self.request(Pending::Other, "pause", json!({ "threadId": thread }));
        true
    }

    /// Ends the session and the program.
    pub fn stop(&mut self) {
        if self.is_live() {
            self.request(
                Pending::Other,
                "disconnect",
                json!({ "terminateDebuggee": true, "restart": false }),
            );
        }
    }

    /// Looks at another frame of the call stack.
    pub fn select_frame(&mut self, index: usize) {
        let Some(f) = self.frames.get(index) else {
            return;
        };
        let id = f.id;
        self.frame = Some(index);
        self.scopes.clear();
        self.variables.clear();
        self.request(Pending::Scopes(id), "scopes", json!({ "frameId": id }));
        self.evaluate_watches();
    }

    /// Fetches a variable's children (once).
    pub fn expand(&mut self, reference: i64) {
        if reference == 0
            || self.variables.contains_key(&reference)
            || self
                .pending
                .values()
                .any(|p| *p == Pending::Variables(reference))
        {
            return;
        }
        self.request(
            Pending::Variables(reference),
            "variables",
            json!({ "variablesReference": reference }),
        );
    }

    /// Evaluates an expression for the console, in the current frame.
    pub fn evaluate(&mut self, expression: &str) {
        self.push_console(ConsoleKind::Input, format!("> {expression}\n"));
        let frame = self.current_frame().map(|f| f.id);
        self.request(
            Pending::Evaluate(EvalFor::Console),
            "evaluate",
            json!({ "expression": expression, "frameId": frame, "context": "repl" }),
        );
    }

    pub fn add_watch(&mut self, expression: &str) {
        self.watches.push(Watch {
            expression: expression.to_owned(),
            value: None,
            reference: 0,
        });
        if self.is_stopped() {
            self.evaluate_watch(self.watches.len() - 1);
        }
    }

    pub fn remove_watch(&mut self, index: usize) {
        if index < self.watches.len() {
            self.watches.remove(index);
            // Answers on their way refer to old indexes.
            self.pending
                .retain(|_, p| !matches!(p, Pending::Evaluate(EvalFor::Watch(_))));
            if self.is_stopped() {
                self.evaluate_watches();
            }
        }
    }

    fn evaluate_watches(&mut self) {
        for i in 0..self.watches.len() {
            self.evaluate_watch(i);
        }
    }

    fn evaluate_watch(&mut self, i: usize) {
        let frame = self.current_frame().map(|f| f.id);
        let expression = self.watches[i].expression.clone();
        self.request(
            Pending::Evaluate(EvalFor::Watch(i)),
            "evaluate",
            json!({ "expression": expression, "frameId": frame, "context": "watch" }),
        );
    }

    /// One source line can compile to several breakpoint locations (a macro call, say).
    /// Continue stopping at another location of the same breakpoint, on the same line of
    /// the same function, isn't what anyone means, so keep going. A loop that hits the
    /// same location again still stops.
    fn skip_same_line_location(&mut self) -> bool {
        let Some((loc, from)) = self.stop_location.take() else {
            return false;
        };
        let Some(top) = self.frames.first().map(|f| (f.name.clone(), f.line)) else {
            return false;
        };
        // The same location again is a loop coming round: a real stop.
        let again = self
            .last_stop
            .as_ref()
            .is_some_and(|(l, f, n)| *l == loc && (f, n) == (&top.0, &top.1));
        let skip = from.as_ref() == Some(&top) && !again;
        if !skip {
            self.last_stop = Some((loc, top.0, top.1));
            return false;
        }
        self.resume("continue");
        true
    }

    fn running(&mut self) {
        self.state = State::Running;
        self.frames.clear();
        self.frame = None;
        self.scopes.clear();
        self.variables.clear();
    }

    fn push_console(&mut self, kind: ConsoleKind, text: String) {
        if let Some(last) = self.console.last_mut()
            && last.kind == kind
            && !last.text.ends_with('\n')
        {
            last.text.push_str(&text);
            return;
        }
        if self.console.len() > 5000 {
            self.console.drain(..1000);
        }
        self.console.push(ConsoleEntry { kind, text });
    }

    // ── what the adapter says ──

    pub fn handle(&mut self, msg: Message) {
        match msg {
            Message::Response {
                request_seq,
                command,
                success,
                message,
                body,
            } => {
                let why = self.pending.remove(&request_seq).unwrap_or(Pending::Other);
                self.on_response(why, &command, success, message, body);
            }
            Message::Event { event, body } => self.on_event(&event, body),
            Message::Request { seq, command, .. } => {
                // We don't offer a terminal; the adapter falls back to its own.
                let s = self.next_seq();
                self.outbox.push(Outgoing::Response {
                    seq: s,
                    request_seq: seq,
                    command,
                    success: false,
                });
            }
            Message::Stderr(line) => {
                self.push_console(ConsoleKind::Debugger, format!("{line}\n"));
            }
            Message::Exited => {
                if self.state != State::Terminated {
                    self.state = State::Terminated;
                    self.frames.clear();
                    self.frame = None;
                }
            }
        }
    }

    fn on_response(
        &mut self,
        why: Pending,
        command: &str,
        success: bool,
        message: Option<String>,
        body: Value,
    ) {
        if !success {
            let msg = message.unwrap_or_else(|| format!("{command} failed"));
            match why {
                Pending::Evaluate(EvalFor::Console) => {
                    self.push_console(ConsoleKind::Error, format!("{msg}\n"));
                }
                Pending::Evaluate(EvalFor::Watch(i)) => {
                    if let Some(w) = self.watches.get_mut(i) {
                        w.value = Some(Err(msg));
                        w.reference = 0;
                    }
                }
                Pending::Launch | Pending::Initialize => {
                    self.error = Some(msg.clone());
                    self.push_console(ConsoleKind::Error, format!("{msg}\n"));
                    self.state = State::Terminated;
                }
                // A step or continue the program had already moved past isn't news.
                _ => {}
            }
            return;
        }
        match why {
            Pending::Initialize => {
                self.capabilities = body;
                let c = &self.config;
                let env: Vec<String> = c.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
                let launch = json!({
                    "program": c.program, "args": c.args, "cwd": c.cwd, "env": env,
                    "initCommands": c.init_commands, "stopOnEntry": false,
                });
                self.request(Pending::Launch, "launch", launch);
            }
            Pending::Launch => {
                if self.state == State::Starting {
                    self.state = State::Running;
                }
            }
            Pending::SetBreakpoints(path) => {
                let answers = body["breakpoints"].as_array().cloned().unwrap_or_default();
                if let Some(bps) = self.breakpoints.get_mut(&path) {
                    for (bp, a) in bps.iter_mut().zip(answers) {
                        apply_breakpoint(bp, &a);
                    }
                }
            }
            Pending::Threads => {
                self.threads = body["threads"]
                    .as_array()
                    .map(|ts| {
                        ts.iter()
                            .map(|t| {
                                (
                                    t["id"].as_i64().unwrap_or(0),
                                    t["name"].as_str().unwrap_or_default().to_owned(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Pending::StackTrace => {
                self.frames = body["stackFrames"]
                    .as_array()
                    .map(|fs| fs.iter().map(frame_from).collect())
                    .unwrap_or_default();
                if self.skip_same_line_location() {
                    return;
                }
                // Stopped by the `rust_panic` breakpoint: say so.
                if self
                    .frames
                    .iter()
                    .take(4)
                    .any(|f| f.name.contains("rust_panic"))
                    && let State::Stopped { description, .. } = &mut self.state
                {
                    *description = Some("The program panicked".into());
                }
                let mine = self.frames.iter().position(|f| {
                    f.path
                        .as_deref()
                        .is_some_and(|p| p.starts_with(&self.config.project_root))
                });
                let pick = mine
                    .or_else(|| self.frames.iter().position(|f| f.path.is_some()))
                    .unwrap_or(0);
                if !self.frames.is_empty() {
                    self.select_frame(pick);
                }
            }
            Pending::Scopes(frame) => {
                if self.current_frame().map(|f| f.id) != Some(frame) {
                    return; // another frame was picked meanwhile
                }
                self.scopes = body["scopes"]
                    .as_array()
                    .map(|ss| {
                        ss.iter()
                            .map(|s| Scope {
                                name: s["name"].as_str().unwrap_or_default().to_owned(),
                                reference: s["variablesReference"].as_i64().unwrap_or(0),
                                expensive: s["expensive"].as_bool().unwrap_or(false),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Locals straight away; the rest when opened.
                if let Some(first) = self.scopes.first().map(|s| s.reference) {
                    self.expand(first);
                }
            }
            Pending::Variables(reference) => {
                let vars = body["variables"]
                    .as_array()
                    .map(|vs| {
                        vs.iter()
                            .map(|v| Variable {
                                name: v["name"].as_str().unwrap_or_default().to_owned(),
                                value: v["value"].as_str().unwrap_or_default().to_owned(),
                                ty: v["type"].as_str().map(str::to_owned),
                                reference: v["variablesReference"].as_i64().unwrap_or(0),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                self.variables.insert(reference, vars);
            }
            Pending::Evaluate(EvalFor::Console) => {
                let result = body["result"].as_str().unwrap_or_default().to_owned();
                self.push_console(ConsoleKind::Result, format!("{result}\n"));
            }
            Pending::Evaluate(EvalFor::Watch(i)) => {
                if let Some(w) = self.watches.get_mut(i) {
                    w.value = Some(Ok(body["result"].as_str().unwrap_or_default().to_owned()));
                    w.reference = body["variablesReference"].as_i64().unwrap_or(0);
                }
            }
            Pending::FunctionBreakpoints | Pending::ConfigurationDone | Pending::Other => {}
        }
    }

    fn on_event(&mut self, event: &str, body: Value) {
        match event {
            "initialized" => {
                self.configured = true;
                let paths: Vec<PathBuf> = self.breakpoints.keys().cloned().collect();
                for p in paths {
                    self.send_breakpoints(&p);
                }
                if self.config.stop_on_panic {
                    self.request(
                        Pending::FunctionBreakpoints,
                        "setFunctionBreakpoints",
                        json!({ "breakpoints": [{ "name": "rust_panic" }] }),
                    );
                }
                self.request(
                    Pending::Other,
                    "setExceptionBreakpoints",
                    json!({ "filters": [] }),
                );
                self.request(Pending::ConfigurationDone, "configurationDone", json!({}));
            }
            "stopped" => {
                let thread = body["threadId"]
                    .as_i64()
                    .or_else(|| self.threads.first().map(|t| t.0))
                    .unwrap_or(1);
                let reason = body["reason"].as_str().unwrap_or("pause").to_owned();
                let mut description = body["description"]
                    .as_str()
                    .or_else(|| body["text"].as_str())
                    .map(str::to_owned);
                if body["hitBreakpointIds"]
                    .as_array()
                    .is_some_and(|ids| !ids.is_empty())
                    && reason == "function breakpoint"
                {
                    description = Some("The program panicked".into());
                } else if reason == "breakpoint" {
                    // LLDB's "breakpoint 1.2": keep the location id, show nothing.
                    self.stop_location = description
                        .as_deref()
                        .and_then(|d| d.strip_prefix("breakpoint "))
                        .map(|loc| (loc.to_owned(), self.continuing.clone()));
                    description = None;
                } else {
                    self.stop_location = None;
                }
                self.continuing = None;
                self.running(); // clears the old stack
                self.state = State::Stopped {
                    thread,
                    reason,
                    description,
                };
                self.stops += 1;
                self.request(Pending::Threads, "threads", json!({}));
                self.request(
                    Pending::StackTrace,
                    "stackTrace",
                    json!({ "threadId": thread, "startFrame": 0, "levels": 64 }),
                );
            }
            "continued" => {
                if self.state != State::Terminated {
                    self.running();
                }
            }
            "exited" => self.exit_code = body["exitCode"].as_i64(),
            "terminated" => {
                self.state = State::Terminated;
                self.frames.clear();
                self.frame = None;
                let msg = match self.exit_code {
                    Some(c) => format!("The program exited with code {c}.\n"),
                    None => "The debug session ended.\n".to_owned(),
                };
                self.push_console(ConsoleKind::Debugger, msg);
            }
            "output" => {
                let text = body["output"].as_str().unwrap_or_default().to_owned();
                let kind = match body["category"].as_str() {
                    Some("stdout") => ConsoleKind::Stdout,
                    Some("stderr") => ConsoleKind::Stderr,
                    Some("telemetry") => return,
                    _ => ConsoleKind::Debugger,
                };
                if kind == ConsoleKind::Debugger {
                    // LLDB has no Rust plugin, but the toolchain's formatters cover it:
                    // the warning only misleads.
                    if text.contains("no plugin for the language \"rust\"") {
                        return;
                    }
                    // Debugger messages are whole lines, with or without the newline.
                    let mut text = text;
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                    self.push_console(kind, text);
                    return;
                }
                self.push_console(kind, text);
            }
            "breakpoint" => {
                let b = &body["breakpoint"];
                if let Some(id) = b["id"].as_i64() {
                    for bp in self.breakpoints.values_mut().flatten() {
                        if bp.id == Some(id) {
                            apply_breakpoint(bp, b);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn apply_breakpoint(bp: &mut Breakpoint, a: &Value) {
    bp.id = a["id"].as_i64().or(bp.id);
    bp.verified = a["verified"].as_bool().unwrap_or(false);
    bp.actual_line = a["line"]
        .as_u64()
        .map(|l| l as usize)
        .filter(|&l| l != bp.line);
    bp.message = a["message"].as_str().map(str::to_owned);
}

fn frame_from(f: &Value) -> Frame {
    Frame {
        id: f["id"].as_i64().unwrap_or(0),
        name: f["name"].as_str().unwrap_or_default().to_owned(),
        path: f["source"]["path"]
            .as_str()
            // Frames without source get names like `target/debug/app`main`: not a file.
            .filter(|p| Path::new(p).is_absolute() && !p.contains('`'))
            .map(PathBuf::from),
        line: f["line"].as_u64().unwrap_or(0) as usize,
        column: f["column"].as_u64().unwrap_or(0) as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requests(s: &mut Session) -> Vec<(i64, String, Value)> {
        s.take_outbox()
            .into_iter()
            .filter_map(|o| match o {
                Outgoing::Request {
                    seq,
                    command,
                    arguments,
                } => Some((seq, command, arguments)),
                Outgoing::Response { .. } => None,
            })
            .collect()
    }

    fn ok(seq: i64, command: &str, body: Value) -> Message {
        Message::Response {
            request_seq: seq,
            command: command.into(),
            success: true,
            message: None,
            body,
        }
    }

    fn event(event: &str, body: Value) -> Message {
        Message::Event {
            event: event.into(),
            body,
        }
    }

    fn started() -> Session {
        let mut s = Session::new(LaunchConfig {
            program: "/p/target/debug/app".into(),
            cwd: "/p".into(),
            project_root: "/p".into(),
            stop_on_panic: true,
            ..Default::default()
        });
        s.set_breakpoints(Path::new("/p/src/main.rs"), &[3, 7]);
        let r = requests(&mut s);
        assert_eq!(r.len(), 1, "breakpoints wait for `initialized`: {r:?}");
        assert_eq!(r[0].1, "initialize");
        s.handle(ok(
            r[0].0,
            "initialize",
            json!({ "supportsConfigurationDoneRequest": true }),
        ));
        let r = requests(&mut s);
        assert_eq!(r[0].1, "launch");
        assert_eq!(r[0].2["program"], "/p/target/debug/app");
        s.handle(event("initialized", Value::Null));
        let r = requests(&mut s);
        let cmds: Vec<&str> = r.iter().map(|x| x.1.as_str()).collect();
        assert_eq!(
            cmds,
            [
                "setBreakpoints",
                "setFunctionBreakpoints",
                "setExceptionBreakpoints",
                "configurationDone"
            ]
        );
        assert_eq!(r[0].2["breakpoints"], json!([{ "line": 3 }, { "line": 7 }]));
        s.handle(ok(
            r[0].0,
            "setBreakpoints",
            json!({ "breakpoints": [{ "id": 1, "verified": true, "line": 3 }, { "id": 2, "verified": true, "line": 8 }] }),
        ));
        s.handle(ok(r[3].0, "configurationDone", Value::Null));
        s.handle(ok(2, "launch", Value::Null));
        s
    }

    #[test]
    fn starts_and_sets_breakpoints() {
        let s = started();
        assert_eq!(s.state, State::Running);
        let bps = &s.breakpoints[Path::new("/p/src/main.rs")];
        assert!(bps.iter().all(|b| b.verified));
        assert_eq!(bps[1].actual_line, Some(8));
    }

    #[test]
    fn a_stop_loads_the_stack_and_locals() {
        let mut s = started();
        s.handle(event(
            "stopped",
            json!({ "reason": "breakpoint", "threadId": 5, "hitBreakpointIds": [1] }),
        ));
        assert!(s.is_stopped());
        let r = requests(&mut s);
        let st = r.iter().find(|x| x.1 == "stackTrace").unwrap();
        assert_eq!(st.2["threadId"], 5);
        // The innermost frame is in std: the first frame in the project gets picked.
        s.handle(ok(st.0, "stackTrace", json!({ "stackFrames": [
            { "id": 100, "name": "core::panicking::panic", "source": { "path": "/rustc/abc/library/core/src/panicking.rs" }, "line": 70, "column": 1 },
            { "id": 101, "name": "app::main", "source": { "path": "/p/src/main.rs" }, "line": 3, "column": 5 }
        ] })));
        assert_eq!(s.current_frame().unwrap().id, 101);
        let r = requests(&mut s);
        let sc = r.iter().find(|x| x.1 == "scopes").unwrap();
        assert_eq!(sc.2["frameId"], 101);
        s.handle(ok(
            sc.0,
            "scopes",
            json!({ "scopes": [
            { "name": "Locals", "variablesReference": 7, "expensive": false },
            { "name": "Registers", "variablesReference": 8, "expensive": false }
        ] }),
        ));
        let r = requests(&mut s);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].2["variablesReference"], 7);
        s.handle(ok(
            r[0].0,
            "variables",
            json!({ "variables": [
            { "name": "count", "value": "3", "type": "i32", "variablesReference": 0 },
            { "name": "names", "value": "size=2", "type": "Vec<String>", "variablesReference": 9 }
        ] }),
        ));
        assert_eq!(s.variables[&7][0].value, "3");
        s.expand(9);
        s.expand(9); // asked once only
        assert_eq!(requests(&mut s).len(), 1);

        // Stepping clears the stack until the next stop.
        assert!(s.step_over());
        assert_eq!(s.state, State::Running);
        assert!(s.frames.is_empty());
        assert_eq!(requests(&mut s)[0].1, "next");
        assert!(!s.step_over(), "can't step while running");
    }

    #[test]
    fn continue_skips_other_locations_on_the_same_line() {
        let mut s = started();
        let stop = |s: &mut Session, loc: &str, line: usize| {
            s.handle(event("stopped", json!({ "reason": "breakpoint", "threadId": 1, "description": format!("breakpoint {loc}") })));
            let r = requests(s);
            let st = r.iter().find(|x| x.1 == "stackTrace").unwrap().0;
            s.handle(ok(st, "stackTrace", json!({ "stackFrames": [{ "id": 1, "name": "app::main", "source": { "path": "/p/src/main.rs" }, "line": line }] })));
            requests(s)
        };
        stop(&mut s, "2.1", 29);
        assert!(s.continue_());
        requests(&mut s);
        // Location 2.2 of the same breakpoint, same line: carry on.
        let sent = stop(&mut s, "2.2", 29);
        assert_eq!(s.state, State::Running);
        assert!(sent.iter().any(|x| x.1 == "continue"), "{sent:?}");
        // The loop comes round to the first location again: a real stop.
        stop(&mut s, "2.1", 29);
        assert!(s.is_stopped());
        // Stepping onto another location isn't skipped either.
        assert!(s.step_over());
        requests(&mut s);
        stop(&mut s, "2.1", 29);
        assert!(s.is_stopped());
        // But Continue from a step stop on that line skips the line's other location.
        s.handle(event("stopped", json!({ "reason": "step", "threadId": 1 })));
        let r = requests(&mut s);
        let st = r.iter().find(|x| x.1 == "stackTrace").unwrap().0;
        s.handle(ok(st, "stackTrace", json!({ "stackFrames": [{ "id": 1, "name": "app::main", "source": { "path": "/p/src/main.rs" }, "line": 29 }] })));
        requests(&mut s);
        assert!(s.continue_());
        requests(&mut s);
        stop(&mut s, "2.2", 29);
        assert_eq!(s.state, State::Running);
    }

    #[test]
    fn a_panic_is_named() {
        let mut s = started();
        s.handle(event(
            "stopped",
            json!({ "reason": "breakpoint", "threadId": 1, "description": "breakpoint 3.1" }),
        ));
        let r = requests(&mut s);
        let st = r.iter().find(|x| x.1 == "stackTrace").unwrap().0;
        s.handle(ok(
            st,
            "stackTrace",
            json!({ "stackFrames": [
            { "id": 1, "name": "std::panicking::rust_panic", "line": 1 },
            { "id": 2, "name": "app::main", "source": { "path": "/p/src/main.rs" }, "line": 32 }
        ] }),
        ));
        assert!(
            matches!(&s.state, State::Stopped { description: Some(d), .. } if d.contains("panicked"))
        );
        assert_eq!(s.current_frame().unwrap().line, 32);
    }

    #[test]
    fn console_watches_and_output() {
        let mut s = started();
        s.handle(event(
            "output",
            json!({ "category": "stdout", "output": "hel" }),
        ));
        s.handle(event(
            "output",
            json!({ "category": "stdout", "output": "lo\n" }),
        ));
        s.handle(event(
            "output",
            json!({ "category": "stdout", "output": "again\n" }),
        ));
        assert_eq!(s.console.len(), 2);
        assert_eq!(s.console[0].text, "hello\n");

        s.handle(event("stopped", json!({ "reason": "step", "threadId": 1 })));
        let r = requests(&mut s);
        let st = r.iter().find(|x| x.1 == "stackTrace").unwrap();
        s.handle(ok(st.0, "stackTrace", json!({ "stackFrames": [{ "id": 1, "name": "main", "source": { "path": "/p/src/main.rs" }, "line": 4 }] })));
        requests(&mut s);
        s.add_watch("count * 2");
        let r = requests(&mut s);
        assert_eq!(r[0].2["context"], "watch");
        s.handle(ok(
            r[0].0,
            "evaluate",
            json!({ "result": "6", "variablesReference": 0 }),
        ));
        assert_eq!(s.watches[0].value, Some(Ok("6".into())));

        s.evaluate("nope");
        let r = requests(&mut s);
        s.handle(Message::Response {
            request_seq: r[0].0,
            command: "evaluate".into(),
            success: false,
            message: Some("use of undeclared identifier 'nope'".into()),
            body: Value::Null,
        });
        let last = s.console.last().unwrap();
        assert_eq!(last.kind, ConsoleKind::Error);
        assert!(last.text.contains("undeclared"));

        s.handle(event("exited", json!({ "exitCode": 0 })));
        s.handle(event("terminated", Value::Null));
        assert_eq!(s.state, State::Terminated);
        assert!(
            s.console
                .last()
                .unwrap()
                .text
                .contains("exited with code 0")
        );
    }

    #[test]
    fn launch_failure_ends_the_session() {
        let mut s = Session::new(LaunchConfig::default());
        let r = requests(&mut s);
        s.handle(ok(r[0].0, "initialize", json!({})));
        let r = requests(&mut s);
        s.handle(Message::Response {
            request_seq: r[0].0,
            command: "launch".into(),
            success: false,
            message: Some("executable doesn't exist".into()),
            body: Value::Null,
        });
        assert_eq!(s.state, State::Terminated);
        assert_eq!(s.error.as_deref(), Some("executable doesn't exist"));
    }

    #[test]
    fn reverse_requests_are_declined() {
        let mut s = Session::new(LaunchConfig::default());
        s.take_outbox();
        s.handle(Message::Request {
            seq: 9,
            command: "runInTerminal".into(),
            arguments: Value::Null,
        });
        assert!(matches!(
            s.take_outbox()[..],
            [Outgoing::Response {
                request_seq: 9,
                success: false,
                ..
            }]
        ));
    }
}
