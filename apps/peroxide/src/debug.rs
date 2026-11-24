//! The debugger in the IDE ([ADR 0012](../../../docs/adr/0012-debugger.md)): building
//! the program, starting lldb-dap, breakpoints from the editors' gutters, following the
//! stopped line, and the Debug and Debug Console panes. `perox-dap` does the protocol.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use perox_build::Kind;
use perox_build::testing::Binary;
use perox_core::{Access, CommandBus, CommandError, CommandSpec, Invocation};
use perox_dap::session::{ConsoleKind, Variable};
use perox_dap::{Debugger, LaunchConfig, State, adapter};
use perox_icons::Icon;
use serde_json::{Value, json};

use crate::commands::AppState;
use crate::docs::DocId;
use crate::ide::LayoutRequest;
use crate::layout::Pane;

/// What to debug once the build is done.
#[derive(Debug, Clone)]
enum Launch {
    /// The project's binary (`target`: a binary's name, when there are several).
    Program {
        target: Option<String>,
        args: Vec<String>,
    },
    /// One test, in its harness.
    Test { binary: Binary, name: String },
}

#[derive(Debug, Default)]
pub struct Debug {
    pub debugger: Option<Debugger>,
    /// Breakpoints by file, 1-based lines. Open editors are the truth for their files and
    /// are copied in every frame; this keeps the rest (and is saved with the project).
    pub breakpoints: BTreeMap<PathBuf, Vec<usize>>,
    /// Documents whose editors have been given their breakpoints.
    seeded: HashSet<DocId>,
    pending: Option<Launch>,
    /// The (stop count, frame) last shown in an editor.
    shown: Option<(u64, usize)>,
    pub console_input: String,
    pub watch_input: String,
    /// Why the last start failed.
    pub error: Option<String>,
    /// Show the standard library's frames in the call stack instead of folding them.
    pub show_library_frames: bool,
    /// Rust formatter commands for LLDB (found once).
    init_commands: Option<Vec<String>>,
}

impl Debug {
    pub fn session(&self) -> Option<&perox_dap::Session> {
        self.debugger.as_ref().map(|d| &d.session)
    }

    pub fn is_live(&self) -> bool {
        self.session().is_some_and(|s| s.is_live())
    }

    pub fn is_stopped(&self) -> bool {
        self.session().is_some_and(|s| s.is_stopped())
    }

    /// Building before a debug session.
    pub fn is_preparing(&self) -> bool {
        self.pending.is_some()
    }

    /// Forgets the project's session state (another project was opened).
    pub fn reset(&mut self, breakpoints: BTreeMap<PathBuf, Vec<usize>>) {
        *self = Self {
            breakpoints,
            ..Default::default()
        };
    }

    fn with_session(&mut self, f: impl FnOnce(&mut perox_dap::Session) -> bool) -> bool {
        let Some(d) = &mut self.debugger else {
            return false;
        };
        let did = f(&mut d.session);
        d.flush();
        did
    }
}

fn err(msg: impl Into<String>) -> CommandError {
    CommandError::Failed(msg.into())
}

fn sysroot_commands() -> Vec<String> {
    let Some(cargo) = perox_build::tools::cargo() else {
        return Vec::new();
    };
    let rustc = cargo.with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" });
    let run = |args: &[&str]| {
        std::process::Command::new(&rustc)
            .args(args)
            .envs(perox_build::tools::env())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let Some(sysroot) = run(&["--print", "sysroot"]) else {
        return Vec::new();
    };
    let vv = run(&["-vV"]).unwrap_or_default();
    adapter::rust_init_commands(Path::new(sysroot.trim()), adapter::commit_hash(&vv))
}

/// Starts debugging: builds, then launches under the debugger when the build is done.
fn start(s: &mut AppState, launch: Launch) -> Result<Value, CommandError> {
    if s.project.is_none() {
        return Err(err("No project is open."));
    }
    if adapter::find(s.settings.debugger_adapter.as_deref()).is_none() {
        let msg = format!(
            "No debugger (lldb-dap) was found. {}",
            adapter::install_hint()
        );
        s.debug.error = Some(msg.clone());
        return Err(err(msg));
    }
    if let Some(d) = &mut s.debug.debugger {
        d.kill();
    }
    s.debug.debugger = None;
    s.debug.error = None;
    let (kind, args) = match &launch {
        Launch::Program { target, .. } => {
            let mut args = Vec::new();
            if let Some(t) = target {
                args.extend(["--bin".to_owned(), t.clone()]);
            }
            (Kind::Build, args)
        }
        Launch::Test { binary, .. } => {
            let mut args = binary.cargo_args(None);
            args.push("--no-run".into());
            (Kind::Test, args)
        }
    };
    crate::ide::start_build_with(s, kind, &args)?;
    s.debug.pending = Some(launch);
    s.status = "Building for debugging…".into();
    Ok(json!({
        "building": true,
        "hint": "The program starts under the debugger when the build is done; debug.state follows it."
    }))
}

/// A cargo job ended: launch if it was building for the debugger.
pub fn build_finished(s: &mut AppState, ok: bool) {
    let Some(launch) = s.debug.pending.take() else {
        return;
    };
    if !ok {
        s.debug.error = Some("The build failed, so debugging didn't start.".into());
        s.status = "Build failed: see Problems".into();
        return;
    }
    if let Err(e) = launch_now(s, launch) {
        s.status = e.clone();
        s.debug.error = Some(e);
    }
}

fn launch_now(s: &mut AppState, launch: Launch) -> Result<(), String> {
    let root = s
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or("No project is open.")?;
    let adapter = adapter::find(s.settings.debugger_adapter.as_deref()).ok_or_else(|| {
        format!(
            "No debugger (lldb-dap) was found. {}",
            adapter::install_hint()
        )
    })?;
    let (program, args, cwd) = match launch {
        Launch::Program { target, args } => {
            let bins: Vec<&perox_build::Artifact> = s
                .build
                .artifacts
                .iter()
                .filter(|a| a.kind == "bin" && !a.test)
                .collect();
            let exe = match (&target, bins.as_slice()) {
                (Some(t), _) => bins.iter().find(|a| &a.target == t),
                (None, [one]) => Some(one),
                (None, many) => {
                    // The package named like the project, else the first.
                    let name = s.project.as_ref().map(|p| p.name.replace('-', "_"));
                    many.iter()
                        .find(|a| Some(a.target.replace('-', "_")) == name)
                        .or(many.first())
                }
            }
            .ok_or("There's no binary to debug (is this a library? Debug a test instead).")?;
            (exe.executable.clone(), args, root.clone())
        }
        Launch::Test { binary, name } => {
            // cargo runs a test with its package's folder as the working directory.
            let cwd = binary
                .src
                .ancestors()
                .find(|d| d.join("Cargo.toml").is_file())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.clone());
            let args = vec![
                name,
                "--exact".into(),
                "--nocapture".into(),
                "--test-threads=1".into(),
            ];
            (binary.exe.clone(), args, cwd)
        }
    };
    let init_commands = s
        .debug
        .init_commands
        .get_or_insert_with(sysroot_commands)
        .clone();
    let config = LaunchConfig {
        program,
        args,
        cwd,
        env: Vec::new(),
        init_commands,
        stop_on_panic: true,
        project_root: root,
    };
    let ctx = s.ctx.clone();
    let mut d = Debugger::start(&adapter, config, move || ctx.request_repaint())
        .map_err(|e| format!("Couldn't start {}: {e}", adapter.display()))?;
    for (path, lines) in &s.debug.breakpoints {
        d.session.set_breakpoints(path, lines);
    }
    d.flush();
    s.debug.debugger = Some(d);
    s.debug.shown = None;
    s.status = "Debugging".into();
    s.layout_requests.push(LayoutRequest::ShowPane(Pane::Debug));
    s.layout_requests
        .push(LayoutRequest::ShowPane(Pane::DebugConsole));
    Ok(())
}

/// Every frame: messages from the debugger, breakpoints from the editors, and the
/// stopped line in the right editor.
pub fn update(s: &mut AppState) {
    if let Some(d) = &mut s.debug.debugger {
        d.poll();
        if !d.session.is_live() && d.session.state == State::Terminated {
            // Done: free the adapter but keep the session for its console.
            if !s.debug.shown.is_some_and(|(n, _)| n == u64::MAX) {
                d.kill();
                s.debug.shown = Some((u64::MAX, 0));
                s.status = match d.session.exit_code {
                    Some(c) => format!("Debugging ended (exit code {c})"),
                    None => "Debugging ended".into(),
                };
            }
        }
    }
    sync_breakpoints(s);
    follow_placement(s);
    follow_stop(s);
}

/// A breakpoint on a line with no code (blank, a comment) lands on the next line that
/// has some: move it there, as the debugger did.
fn follow_placement(s: &mut AppState) {
    let Some(ses) = s.debug.session() else {
        return;
    };
    let mut moves: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (path, bps) in &ses.breakpoints {
        let Some(lines) = s.debug.breakpoints.get(path) else {
            continue;
        };
        let mut new = lines.clone();
        for b in bps {
            if let Some(actual) = b.actual_line
                && let Some(i) = new.iter().position(|&l| l == b.line)
            {
                new[i] = actual;
            }
        }
        new.sort_unstable();
        new.dedup();
        if new != *lines {
            moves.push((path.clone(), new));
        }
    }
    if moves.is_empty() {
        return;
    }
    for (path, lines) in moves {
        set_file_breakpoints(s, &path, lines);
    }
    s.save_session();
}

fn sync_breakpoints(s: &mut AppState) {
    let open: HashSet<DocId> = s.docs.list.iter().map(|d| d.id).collect();
    s.debug.seeded.retain(|id| open.contains(id));
    let mut changed: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    let session = s.debug.debugger.as_ref().map(|d| &d.session);
    for doc in &mut s.docs.list {
        if doc.path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if s.debug.seeded.insert(doc.id) {
            let lines: Vec<usize> = s
                .debug
                .breakpoints
                .get(&doc.path)
                .map(|ls| ls.iter().map(|l| l.saturating_sub(1)).collect())
                .unwrap_or_default();
            doc.editor.set_breakpoints(&lines);
        }
        let now: Vec<usize> = doc.editor.breakpoints().iter().map(|l| l + 1).collect();
        let before = s
            .debug
            .breakpoints
            .get(&doc.path)
            .cloned()
            .unwrap_or_default();
        if now != before {
            changed.push((doc.path.clone(), now));
        }
        // Breakpoints the debugger couldn't place are drawn hollow.
        doc.editor.deco.unverified_breakpoints = session
            .and_then(|ses| ses.breakpoints.get(&doc.path))
            .map(|bps| {
                bps.iter()
                    .filter(|b| b.id.is_some() && !b.verified)
                    .map(|b| b.line.saturating_sub(1))
                    .collect()
            })
            .unwrap_or_default();
    }
    if changed.is_empty() {
        return;
    }
    for (path, lines) in changed {
        set_file_breakpoints(s, &path, lines);
    }
    s.save_session();
}

/// Sets a file's breakpoints everywhere: the map, the live session, an open editor.
fn set_file_breakpoints(s: &mut AppState, path: &Path, lines: Vec<usize>) {
    s.debug.with_session(|ses| {
        ses.set_breakpoints(path, &lines);
        true
    });
    if let Some(doc) = s.docs.list.iter_mut().find(|d| d.path == path) {
        let zero: Vec<usize> = lines.iter().map(|l| l.saturating_sub(1)).collect();
        if doc.editor.breakpoints() != zero {
            doc.editor.set_breakpoints(&zero);
        }
    }
    if lines.is_empty() {
        s.debug.breakpoints.remove(path);
    } else {
        s.debug.breakpoints.insert(path.to_owned(), lines);
    }
}

fn follow_stop(s: &mut AppState) {
    let here = s.debug.session().and_then(|ses| {
        let f = ses.current_frame()?;
        Some((ses.stops, ses.frame?, f.path.clone()?, f.line))
    });
    let Some((stops, frame, path, line)) = here else {
        for doc in &mut s.docs.list {
            doc.editor.deco.exec_line = None;
        }
        if !s.debug.shown.is_some_and(|(n, _)| n == u64::MAX) {
            s.debug.shown = None;
        }
        return;
    };
    if s.debug.shown == Some((stops, frame)) {
        return;
    }
    s.debug.shown = Some((stops, frame));
    if path.is_file() && crate::ide::open_file(s, &path, Some(line), None).is_ok() {
        s.layout_requests
            .push(LayoutRequest::ShowDoc(s.active_doc.unwrap_or(DocId(0))));
    }
    for doc in &mut s.docs.list {
        doc.editor.deco.exec_line = (doc.path == path).then(|| line.saturating_sub(1));
    }
    if let Some(State::Stopped {
        reason,
        description,
        ..
    }) = s.debug.session().map(|x| &x.state)
    {
        s.status = match description {
            Some(d) => format!("Paused: {d}"),
            None => format!("Paused on {reason}"),
        };
    }
}

// ── commands ──

fn state_json(s: &AppState) -> Value {
    let Some(ses) = s.debug.session() else {
        return json!({
            "state": if s.debug.is_preparing() { "building" } else { "not_debugging" },
            "error": s.debug.error,
        });
    };
    let (state, reason, description) = match &ses.state {
        State::Starting => ("starting", None, None),
        State::Running => ("running", None, None),
        State::Stopped {
            reason,
            description,
            ..
        } => ("stopped", Some(reason.clone()), description.clone()),
        State::Terminated => ("terminated", None, None),
    };
    let rel = |p: &Path| {
        s.project
            .as_ref()
            .and_then(|pr| p.strip_prefix(&pr.root).ok())
            .unwrap_or(p)
            .display()
            .to_string()
    };
    let frames: Vec<Value> = ses
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            json!({
                "index": i, "function": f.name,
                "path": f.path.as_deref().map(rel), "line": f.line,
                "current": ses.frame == Some(i),
            })
        })
        .collect();
    let locals: Vec<Value> = ses
        .scopes
        .first()
        .and_then(|sc| ses.variables.get(&sc.reference))
        .map(|vs| vs.iter().map(var_json).collect())
        .unwrap_or_default();
    json!({
        "state": state, "reason": reason, "description": description,
        "exit_code": ses.exit_code, "error": ses.error.as_ref().or(s.debug.error.as_ref()),
        "frames": frames, "locals": locals,
        "watches": ses.watches.iter().map(|w| json!({
            "expression": w.expression,
            "value": match &w.value { Some(Ok(v)) => json!(v), Some(Err(e)) => json!({ "error": e }), None => Value::Null },
        })).collect::<Vec<_>>(),
    })
}

fn var_json(v: &Variable) -> Value {
    json!({ "name": v.name, "value": v.value, "type": v.ty, "reference": v.reference })
}

fn need_session(s: &mut AppState) -> Result<&mut Debugger, CommandError> {
    match &mut s.debug.debugger {
        Some(d) if d.session.is_live() => Ok(d),
        _ => Err(err("Not debugging. Start with debug.start.")),
    }
}

fn step(
    s: &mut AppState,
    f: fn(&mut perox_dap::Session) -> bool,
    what: &str,
) -> Result<Value, CommandError> {
    let d = need_session(s)?;
    if !f(&mut d.session) {
        return Err(err(format!("Can't {what} now: the program isn't paused.")));
    }
    d.flush();
    Ok(json!({ "ok": true, "hint": "debug.state shows where it stops next." }))
}

pub fn register(bus: &mut CommandBus<AppState>) {
    type Handler = fn(&mut AppState, &Invocation) -> Result<Value, CommandError>;
    let commands: Vec<(CommandSpec, Handler)> = vec![
        (
            CommandSpec::new("debug.start", "Start Debugging", Access::Execute)
                .description("Builds the program and runs it under the debugger (lldb-dap), stopping at breakpoints and on panics. With `test` (a full test name from test.list), debugs that one test instead. `target` picks a binary when there are several; `args` go to the program.")
                .params(json!({ "type": "object", "properties": {
                    "test": { "type": "string" }, "harness": { "type": "string" },
                    "target": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } }
                } })),
            |s, inv| {
                if let Some(name) = inv.args.get("test").and_then(Value::as_str) {
                    let harness = inv.args.get("harness").and_then(Value::as_str);
                    let found: Vec<&Binary> = s
                        .build
                        .tests
                        .binaries
                        .iter()
                        .filter(|b| b.tests.iter().any(|t| t == name) && harness.is_none_or(|h| b.key() == h))
                        .collect();
                    let binary = match found.as_slice() {
                        [b] => (*b).clone(),
                        [] => return Err(err("No such test. Run test.discover, then use a name from test.list.")),
                        _ => return Err(err("Several targets have that test: pass `harness` from test.list.")),
                    };
                    return start(s, Launch::Test { binary, name: name.to_owned() });
                }
                let args = inv
                    .args
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                let target = inv.args.get("target").and_then(Value::as_str).map(str::to_owned);
                start(s, Launch::Program { target, args })
            },
        ),
        (
            CommandSpec::new("debug.continue", "Continue", Access::Execute)
                .description("Resumes the paused program. When not debugging, starts debugging (like F5)."),
            |s, _| {
                if !s.debug.is_live() && !s.debug.is_preparing() {
                    return start(s, Launch::Program { target: None, args: Vec::new() });
                }
                step(s, perox_dap::Session::continue_, "continue")
            },
        ),
        (
            CommandSpec::new("debug.step_over", "Step Over", Access::Execute).description("Runs to the next line in this function."),
            |s, _| step(s, perox_dap::Session::step_over, "step"),
        ),
        (
            CommandSpec::new("debug.step_in", "Step Into", Access::Execute).description("Steps into the call on this line."),
            |s, _| step(s, perox_dap::Session::step_in, "step"),
        ),
        (
            CommandSpec::new("debug.step_out", "Step Out", Access::Execute).description("Runs until the current function returns."),
            |s, _| step(s, perox_dap::Session::step_out, "step"),
        ),
        (
            CommandSpec::new("debug.pause", "Pause", Access::Execute).description("Pauses the running program."),
            |s, _| {
                let d = need_session(s)?;
                if !d.session.pause() {
                    return Err(err("The program isn't running."));
                }
                d.flush();
                Ok(json!({ "ok": true }))
            },
        ),
        (
            CommandSpec::new("debug.stop", "Stop Debugging", Access::Execute)
                .description("Ends the debug session and the program. When not debugging, stops the running cargo command."),
            |s, _| {
                if s.debug.is_preparing() {
                    s.debug.pending = None;
                }
                match &mut s.debug.debugger {
                    Some(d) if d.session.is_live() => {
                        d.session.stop();
                        d.flush();
                        Ok(json!({ "stopped": "debugging" }))
                    }
                    _ => {
                        if let Some(job) = &mut s.build.job {
                            job.stop();
                            return Ok(json!({ "stopped": "build" }));
                        }
                        Ok(json!({ "stopped": false, "note": "nothing was running" }))
                    }
                }
            },
        ),
        (
            CommandSpec::new("debug.state", "Debug State", Access::Read)
                .description("Whether the program is running, stopped (why, where), or done; the call stack (`index` for debug.select_frame); the current frame's locals (`reference` > 0 can be opened with debug.variables); watches."),
            |s, _| Ok(state_json(s)),
        ),
        (
            CommandSpec::new("debug.select_frame", "Select Frame", Access::Execute)
                .description("Looks at another frame of the call stack (by `index` from debug.state): its locals and source line.")
                .params(json!({ "type": "object", "required": ["index"], "properties": { "index": { "type": "integer", "minimum": 0 } } })),
            |s, inv| {
                let i = inv.args.get("index").and_then(Value::as_u64).ok_or_else(|| err("`index` is required"))? as usize;
                let d = need_session(s)?;
                if i >= d.session.frames.len() {
                    return Err(err("No such frame."));
                }
                d.session.select_frame(i);
                d.flush();
                Ok(json!({ "ok": true, "hint": "Locals arrive shortly: call debug.state." }))
            },
        ),
        (
            CommandSpec::new("debug.variables", "Variables", Access::Read)
                .description("The children of a variable (fields, elements) by its `reference` from debug.state or an earlier call. The first call fetches them: if `loading` is true, call again.")
                .params(json!({ "type": "object", "required": ["reference"], "properties": { "reference": { "type": "integer", "minimum": 1 } } })),
            |s, inv| {
                let r = inv.args.get("reference").and_then(Value::as_i64).ok_or_else(|| err("`reference` is required"))?;
                let d = need_session(s)?;
                match d.session.variables.get(&r) {
                    Some(vs) => Ok(json!({ "variables": vs.iter().map(var_json).collect::<Vec<_>>() })),
                    None => {
                        d.session.expand(r);
                        d.flush();
                        Ok(json!({ "loading": true }))
                    }
                }
            },
        ),
        (
            CommandSpec::new("debug.evaluate", "Evaluate", Access::Execute)
                .description("Evaluates an expression in the current frame, as typed in the Debug Console; the result is appended to the console (debug.console). LLDB syntax: `count`, `names.len()` won't work but `names` and `*ptr` do.")
                .params(json!({ "type": "object", "required": ["expression"], "properties": { "expression": { "type": "string" } } })),
            |s, inv| {
                let e = inv.str_arg("expression")?.to_owned();
                let d = need_session(s)?;
                d.session.evaluate(&e);
                d.flush();
                Ok(json!({ "ok": true, "hint": "Read the result with debug.console." }))
            },
        ),
        (
            CommandSpec::new("debug.console", "Debug Console", Access::Read)
                .description("The Debug Console: the program's output, the debugger's messages, and evaluated expressions. `lines` (default 50) from the end.")
                .params(json!({ "type": "object", "properties": { "lines": { "type": "integer", "default": 50 } } })),
            |s, inv| {
                let n = inv.args.get("lines").and_then(Value::as_u64).unwrap_or(50) as usize;
                let text: String = s
                    .debug
                    .session()
                    .map(|ses| ses.console.iter().map(|c| c.text.as_str()).collect())
                    .unwrap_or_default();
                let lines: Vec<&str> = text.lines().collect();
                Ok(json!(lines[lines.len().saturating_sub(n)..].join("\n")))
            },
        ),
        (
            CommandSpec::new("debug.watch", "Add Watch", Access::Execute)
                .description("Adds a watch expression, evaluated at every stop (debug.state lists them). `remove` (an index) takes one away instead.")
                .params(json!({ "type": "object", "properties": { "expression": { "type": "string" }, "remove": { "type": "integer", "minimum": 0 } } })),
            |s, inv| {
                let d = need_session(s)?;
                if let Some(i) = inv.args.get("remove").and_then(Value::as_u64) {
                    d.session.remove_watch(i as usize);
                } else {
                    d.session.add_watch(inv.str_arg("expression")?);
                }
                d.flush();
                Ok(json!({ "watches": d.session.watches.len() }))
            },
        ),
        (
            CommandSpec::new("debug.toggle_breakpoint", "Toggle Breakpoint", Access::Execute)
                .description("Adds or removes a breakpoint on a line (1-based). Without arguments, on the cursor's line.")
                .params(json!({ "type": "object", "properties": {
                    "path": { "type": "string" }, "line": { "type": "integer", "minimum": 1 }
                } })),
            |s, inv| {
                let (path, line) = match inv.args.get("line").and_then(Value::as_u64) {
                    Some(l) => (crate::ide::resolve(s, inv, "path")?, l as usize),
                    None => {
                        let id = s.active_doc.ok_or_else(|| err("No editor is active."))?;
                        let doc = s.docs.get(id).ok_or_else(|| err("No editor is active."))?;
                        let head = doc.editor.core.sels.primary().head;
                        (doc.path.clone(), doc.editor.core.doc.pos_of(head).line + 1)
                    }
                };
                let mut lines = s.debug.breakpoints.get(&path).cloned().unwrap_or_default();
                let on = match lines.iter().position(|&l| l == line) {
                    Some(i) => {
                        lines.remove(i);
                        false
                    }
                    None => {
                        lines.push(line);
                        lines.sort_unstable();
                        true
                    }
                };
                set_file_breakpoints(s, &path, lines);
                s.save_session();
                Ok(json!({ "breakpoint": on, "line": line }))
            },
        ),
        (
            CommandSpec::new("debug.breakpoints", "Breakpoints", Access::Read)
                .description("Every breakpoint, and in a session whether the debugger could place it (`verified`)."),
            |s, _| {
                let ses = s.debug.session();
                let list: Vec<Value> = s
                    .debug
                    .breakpoints
                    .iter()
                    .flat_map(|(p, lines)| {
                        lines.iter().map(move |l| (p, *l))
                    })
                    .map(|(p, l)| {
                        let placed = ses
                            .and_then(|x| x.breakpoints.get(p))
                            .and_then(|bs| bs.iter().find(|b| b.line == l));
                        json!({
                            "path": p, "line": l,
                            "verified": placed.map(|b| b.verified),
                            "moved_to": placed.and_then(|b| b.actual_line),
                        })
                    })
                    .collect();
                Ok(json!(list))
            },
        ),
        (
            CommandSpec::new("debug.clear_breakpoints", "Remove All Breakpoints", Access::Execute)
                .description("Removes every breakpoint."),
            |s, _| {
                let paths: Vec<PathBuf> = s.debug.breakpoints.keys().cloned().collect();
                for p in &paths {
                    set_file_breakpoints(s, p, Vec::new());
                }
                s.save_session();
                Ok(json!({ "removed_from_files": paths.len() }))
            },
        ),
        (
            CommandSpec::new("view.debug", "Debug", Access::Read).ui_only(),
            |s, _| {
                s.layout_requests.push(LayoutRequest::ShowPane(Pane::Debug));
                s.layout_requests.push(LayoutRequest::ShowPane(Pane::DebugConsole));
                Ok(Value::Null)
            },
        ),
    ];
    for (spec, h) in commands {
        bus.register(spec, h)
            .expect("debug command ids are valid and unique");
    }
}

// ── panes ──

/// What the panes ask for; the app runs these as commands (or does them directly).
pub enum PaneAction {
    Command(&'static str, Value),
    Open(PathBuf, usize),
}

fn toolbar_button(ui: &mut egui::Ui, enabled: bool, icon: Icon, tip: &str) -> bool {
    ui.add_enabled(enabled, egui::Button::image(icon.image(ui)))
        .on_hover_text(tip)
        .clicked()
}

/// The Debug pane: controls, call stack, variables, watches and breakpoints.
pub fn show(ui: &mut egui::Ui, dbg: &mut Debug, root: Option<&Path>) -> Vec<PaneAction> {
    let mut out = Vec::new();
    let live = dbg.is_live();
    let stopped = dbg.is_stopped();
    let preparing = dbg.is_preparing();
    ui.horizontal(|ui| {
        let cmd = |id| PaneAction::Command(id, json!({}));
        if stopped || !live {
            let tip = if live {
                "Continue (F5)"
            } else {
                "Start Debugging (F5)"
            };
            if toolbar_button(
                ui,
                !preparing,
                if live { Icon::Play } else { Icon::BugPlay },
                tip,
            ) {
                out.push(cmd("debug.continue"));
            }
        } else if toolbar_button(ui, true, Icon::Pause, "Pause") {
            out.push(cmd("debug.pause"));
        }
        for (icon, id, tip) in [
            (Icon::RedoDot, "debug.step_over", "Step Over (F10)"),
            (Icon::ArrowDownToDot, "debug.step_in", "Step Into (F11)"),
            (Icon::ArrowUpFromDot, "debug.step_out", "Step Out (⇧F11)"),
        ] {
            if toolbar_button(ui, stopped, icon, tip) {
                out.push(cmd(id));
            }
        }
        if toolbar_button(ui, live || preparing, Icon::Square, "Stop (⇧F5)") {
            out.push(cmd("debug.stop"));
        }
    });
    let status = match dbg.session().map(|s| &s.state) {
        _ if preparing => "Building…".to_owned(),
        None => "Not debugging. Click in the gutter to set breakpoints.".to_owned(),
        Some(State::Starting) => "Starting…".to_owned(),
        Some(State::Running) => "Running".to_owned(),
        Some(State::Stopped {
            reason,
            description,
            ..
        }) => match description {
            Some(d) => format!("Paused: {d}"),
            None => format!("Paused on {reason}"),
        },
        Some(State::Terminated) => match dbg.session().and_then(|s| s.exit_code) {
            Some(c) => format!("Ended (exit code {c})"),
            None => "Ended".to_owned(),
        },
    };
    ui.weak(status);
    if let Some(e) = &dbg.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("debug_pane")
        .auto_shrink(false)
        .show(ui, |ui| {
            if let Some(d) = &mut dbg.debugger {
                let ses = &mut d.session;
                section(ui, "Call Stack", true, |ui| {
                    if ses.frames.is_empty() {
                        ui.weak(if ses.is_stopped() {
                            "Loading…"
                        } else {
                            "Not paused."
                        });
                    }
                    let mine = |f: &perox_dap::session::Frame| {
                        f.path
                            .as_deref()
                            .zip(root)
                            .is_some_and(|(p, r)| p.starts_with(r))
                    };
                    let mut i = 0;
                    while i < ses.frames.len() {
                        // Runs of library frames fold into one row unless shown.
                        let run = ses.frames[i..].iter().take_while(|f| !mine(f)).count();
                        if run >= 2
                            && !dbg.show_library_frames
                            && ses.frame.is_none_or(|c| c < i || c >= i + run)
                        {
                            let text = format!("{run} library frames");
                            if ui
                                .add(
                                    egui::Button::new(RichText::new(text).weak().italics())
                                        .frame(false),
                                )
                                .on_hover_text("Show them")
                                .clicked()
                            {
                                dbg.show_library_frames = true;
                            }
                            i += run;
                            continue;
                        }
                        let f = &ses.frames[i];
                        let file = f.path.as_deref().map(|p| {
                            let p = root.and_then(|r| p.strip_prefix(r).ok()).unwrap_or(p);
                            format!("{}:{}", p.display(), f.line)
                        });
                        let name = if mine(f) {
                            RichText::new(&f.name)
                        } else {
                            RichText::new(&f.name).weak()
                        };
                        let r = ui
                            .horizontal(|ui| {
                                let r = ui.add(
                                    egui::Button::selectable(ses.frame == Some(i), name).truncate(),
                                );
                                if let Some(file) = &file {
                                    ui.weak(file);
                                }
                                r
                            })
                            .inner;
                        if r.clicked() {
                            out.push(PaneAction::Command(
                                "debug.select_frame",
                                json!({ "index": i }),
                            ));
                        }
                        i += 1;
                    }
                    if dbg.show_library_frames && ui.small_button("Hide library frames").clicked() {
                        dbg.show_library_frames = false;
                    }
                });
                section(ui, "Variables", true, |ui| {
                    if ses.scopes.is_empty() {
                        ui.weak("Not paused.");
                    }
                    let scopes = ses.scopes.clone();
                    for (i, sc) in scopes.iter().enumerate() {
                        let id = ui.make_persistent_id(("scope", &sc.name));
                        let state =
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                id,
                                i == 0,
                            );
                        if state.is_open() {
                            ses.expand(sc.reference);
                        }
                        state
                            .show_header(ui, |ui| ui.strong(&sc.name))
                            .body(|ui| variables(ui, ses, sc.reference, 0));
                    }
                });
                section(ui, "Watch", true, |ui| {
                    let mut remove = None;
                    for (i, w) in ses.watches.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui
                                .add(egui::Button::image(Icon::X.image(ui)).frame(false).small())
                                .on_hover_text("Remove")
                                .clicked()
                            {
                                remove = Some(i);
                            }
                            ui.monospace(&w.expression);
                            match &w.value {
                                Some(Ok(v)) => {
                                    ui.monospace(RichText::new(format!("= {v}")).strong())
                                }
                                Some(Err(e)) => ui.colored_label(ui.visuals().weak_text_color(), e),
                                None => ui.weak("…"),
                            };
                        });
                    }
                    if let Some(i) = remove {
                        ses.remove_watch(i);
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut dbg.watch_input)
                            .hint_text("Add an expression")
                            .desired_width(f32::INFINITY),
                    );
                    if r.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && !dbg.watch_input.trim().is_empty()
                    {
                        ses.add_watch(dbg.watch_input.trim());
                        dbg.watch_input.clear();
                    }
                });
                d.flush();
            }
            section(ui, "Breakpoints", true, |ui| {
                if dbg.breakpoints.is_empty() {
                    ui.weak("None. Click left of a line number to add one.");
                }
                let placed = dbg.debugger.as_ref().map(|d| &d.session.breakpoints);
                for (path, lines) in &dbg.breakpoints {
                    for &l in lines {
                        let shown = root.and_then(|r| path.strip_prefix(r).ok()).unwrap_or(path);
                        let verified = placed
                            .and_then(|p| p.get(path))
                            .and_then(|bs| bs.iter().find(|b| b.line == l))
                            .map(|b| b.verified || b.id.is_none());
                        ui.horizontal(|ui| {
                            let red = egui::Color32::from_rgb(0xe5, 0x14, 0x00);
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                            if verified == Some(false) {
                                ui.painter().circle_stroke(
                                    rect.center(),
                                    4.5,
                                    egui::Stroke::new(1.5, red),
                                );
                            } else {
                                ui.painter().circle_filled(rect.center(), 5.0, red);
                            }
                            if ui.link(format!("{}:{l}", shown.display())).clicked() {
                                out.push(PaneAction::Open(path.clone(), l));
                            }
                            if ui
                                .add(egui::Button::image(Icon::X.image(ui)).frame(false).small())
                                .on_hover_text("Remove")
                                .clicked()
                            {
                                out.push(PaneAction::Command(
                                    "debug.toggle_breakpoint",
                                    json!({ "path": path, "line": l }),
                                ));
                            }
                        });
                    }
                }
            });
        });
    out
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(open)
        .show(ui, body);
}

/// A variable list; children load when opened.
fn variables(ui: &mut egui::Ui, ses: &mut perox_dap::Session, reference: i64, depth: usize) {
    let Some(vars) = ses.variables.get(&reference).cloned() else {
        ui.weak("Loading…");
        return;
    };
    if vars.is_empty() {
        ui.weak("(none)");
    }
    for v in vars {
        let label = |ui: &mut egui::Ui| {
            ui.horizontal(|ui| {
                ui.monospace(
                    RichText::new(&v.name).color(ui.visuals().widgets.inactive.fg_stroke.color),
                );
                // A reference shows as `0x16fdfea70 {id:7, …}`: the fields are what matter.
                let shown = match v.value.split_once(" {") {
                    Some((addr, rest)) if addr.starts_with("0x") => format!("{{{rest}"),
                    _ => v.value.clone(),
                };
                let r =
                    ui.add(egui::Label::new(RichText::new(shown).monospace().strong()).truncate());
                if let Some(t) = &v.ty {
                    r.on_hover_text(t);
                }
            });
        };
        if v.reference > 0 && depth < 12 {
            let id = ui.make_persistent_id(("var", reference, &v.name, depth));
            let state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            );
            if state.is_open() {
                ses.expand(v.reference);
            }
            state
                .show_header(ui, label)
                .body(|ui| variables(ui, ses, v.reference, depth + 1));
        } else {
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                label(ui);
            });
        }
    }
}

/// The Debug Console: program output, debugger messages, and an expression prompt.
pub fn console(ui: &mut egui::Ui, dbg: &mut Debug) {
    let Some(d) = &mut dbg.debugger else {
        ui.weak("Start debugging (F5) to see the program's output here.");
        return;
    };
    let ses = &mut d.session;
    let input_h = 28.0;
    egui::ScrollArea::both()
        .id_salt("debug_console")
        .stick_to_bottom(true)
        .auto_shrink(false)
        .max_height((ui.available_height() - input_h).max(40.0))
        .show(ui, |ui| {
            for e in &ses.console {
                let color = match e.kind {
                    ConsoleKind::Stdout => ui.visuals().text_color(),
                    ConsoleKind::Stderr | ConsoleKind::Error => ui.visuals().error_fg_color,
                    ConsoleKind::Debugger => ui.visuals().weak_text_color(),
                    ConsoleKind::Input => ui.visuals().hyperlink_color,
                    ConsoleKind::Result => ui.visuals().strong_text_color(),
                };
                ui.add(
                    egui::Label::new(
                        RichText::new(e.text.trim_end_matches('\n'))
                            .monospace()
                            .color(color),
                    )
                    .extend(),
                );
            }
        });
    let stopped = ses.is_stopped();
    let r = ui.add_enabled(
        stopped,
        egui::TextEdit::singleline(&mut dbg.console_input)
            .hint_text(if stopped {
                "Evaluate an expression in the current frame"
            } else {
                "Pause the program to evaluate expressions"
            })
            .font(egui::TextStyle::Monospace)
            .desired_width(f32::INFINITY),
    );
    if r.lost_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && !dbg.console_input.trim().is_empty()
    {
        ses.evaluate(dbg.console_input.trim());
        dbg.console_input.clear();
        r.request_focus();
    }
    d.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_for_a_test_uses_its_harness() {
        let b = Binary {
            package: "p".into(),
            target: "p".into(),
            kind: "lib".into(),
            src: PathBuf::from("/w/p/src/lib.rs"),
            exe: PathBuf::from("/w/target/debug/deps/p-1"),
            tests: vec!["t::a".into()],
        };
        let mut args = b.cargo_args(None);
        args.push("--no-run".into());
        assert_eq!(args, ["-p", "p", "--lib", "--no-run"]);
    }
}
