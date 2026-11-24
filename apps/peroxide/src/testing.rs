//! The Test Explorer: every test in the project, grouped by crate target and module,
//! with its last result. Tests are found with `cargo test --no-run` plus each harness's
//! `--list`, and run through cargo like any build, so results stream into the tree.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, RichText};
use perox_build::Kind;
use perox_build::testing::{Binary, Outcome, RunParser};
use perox_core::{Access, CommandBus, CommandError, CommandSpec};
use perox_icons::Icon;
use serde_json::{Value, json};

use crate::commands::AppState;
use crate::ide::LayoutRequest;
use crate::layout::Pane;

/// A test: its harness's key ([`Binary::key`], empty for doc tests) and full name.
pub type TestId = (String, String);

#[derive(Debug, Default)]
pub struct Tests {
    pub binaries: Vec<Binary>,
    /// The last outcome of each test that has run.
    pub results: HashMap<TestId, Outcome>,
    /// Captured output of failed tests.
    pub output: HashMap<TestId, String>,
    /// Tests in the run that's going on (shown as running until they report).
    pub pending: HashSet<TestId>,
    /// Whether the current cargo job is a test run we're reading.
    parser: Option<RunParser>,
    discovering: Option<Receiver<Result<perox_build::testing::Discovery, String>>>,
    /// Why the last discovery failed.
    pub error: Option<String>,
    pub filter: String,
    pub selected: Option<TestId>,
    /// Discovery has run at least once for this project.
    pub discovered: bool,
}

/// Which tests a run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    /// Every test in one harness.
    Binary(String),
    /// Tests in one harness whose names start with `prefix` (a module), or one test.
    Prefix {
        binary: String,
        prefix: String,
    },
    Test(TestId),
    Failed,
}

impl Tests {
    pub fn is_discovering(&self) -> bool {
        self.discovering.is_some()
    }

    /// Starts finding tests in the background.
    pub fn discover(&mut self, root: PathBuf, ctx: &egui::Context) {
        let Some(cargo) = perox_build::tools::cargo() else {
            self.error = Some("cargo isn't installed.".into());
            return;
        };
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(perox_build::testing::discover(&cargo, &root));
            ctx.request_repaint();
        });
        self.discovering = Some(rx);
    }

    /// Takes finished discovery. True if the list changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.discovering else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.discovering = None;
                self.discovered = true;
                match result {
                    Ok(found) => {
                        self.binaries = found.binaries;
                        self.error = found.error.map(|e| format!("Some tests don't build: {e}"));
                    }
                    Err(e) => self.error = Some(e),
                }
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.discovering = None;
                false
            }
            Err(mpsc::TryRecvError::Empty) => false,
        }
    }

    /// Forgets everything (another project was opened).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A test run is starting: `tests` will be shown as running.
    pub fn begin(&mut self, tests: HashSet<TestId>) {
        self.parser = Some(RunParser::default());
        for t in &tests {
            self.results.remove(t);
            self.output.remove(t);
        }
        self.pending = tests;
    }

    /// A line of the running test job's output.
    pub fn feed(&mut self, line: &str) {
        let Some(p) = &mut self.parser else {
            return;
        };
        if let Some(done) = p.feed(line) {
            let id = (done.binary, done.name);
            self.pending.remove(&id);
            self.results.insert(id, done.outcome);
        }
    }

    /// The test job ended. Tests that never reported keep no result.
    pub fn finish(&mut self) {
        if let Some(p) = self.parser.take() {
            self.output.extend(p.output);
        }
        self.pending.clear();
    }

    pub fn is_running(&self) -> bool {
        self.parser.is_some()
    }

    fn binary(&self, key: &str) -> Option<&Binary> {
        self.binaries.iter().find(|b| b.key() == key)
    }

    /// Every known test: listed ones, then ones only seen in results (doc tests, or
    /// tests added since the last discovery).
    pub fn all(&self) -> Vec<TestId> {
        let mut out: Vec<TestId> = Vec::new();
        let mut seen = HashSet::new();
        for b in &self.binaries {
            let key = b.key();
            for t in &b.tests {
                let id = (key.clone(), t.clone());
                seen.insert(id.clone());
                out.push(id);
            }
        }
        let mut extra: Vec<&TestId> = self.results.keys().filter(|k| !seen.contains(*k)).collect();
        extra.sort();
        out.extend(extra.into_iter().cloned());
        out
    }

    pub fn state(&self, id: &TestId) -> State {
        if self.pending.contains(id) {
            State::Running
        } else {
            match self.results.get(id) {
                Some(Outcome::Passed) => State::Passed,
                Some(Outcome::Failed) => State::Failed,
                Some(Outcome::Ignored) => State::Ignored,
                None => State::NotRun,
            }
        }
    }

    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let (mut passed, mut failed, mut ignored, mut not_run) = (0, 0, 0, 0);
        for id in self.all() {
            match self.state(&id) {
                State::Passed => passed += 1,
                State::Failed => failed += 1,
                State::Ignored => ignored += 1,
                State::NotRun | State::Running => not_run += 1,
            }
        }
        (passed, failed, ignored, not_run)
    }

    /// The cargo arguments for a run, and the tests it should cover.
    pub fn plan(&self, scope: &Scope) -> Result<(Vec<String>, HashSet<TestId>), String> {
        let all = self.all();
        match scope {
            Scope::All => Ok((vec!["--no-fail-fast".into()], all.into_iter().collect())),
            Scope::Binary(key) => {
                let b = self.binary(key).ok_or("No such test target.")?;
                let tests = all.into_iter().filter(|(k, _)| k == key).collect();
                Ok((b.cargo_args(None), tests))
            }
            Scope::Prefix { binary, prefix } => {
                let b = self.binary(binary).ok_or("No such test target.")?;
                let tests: HashSet<TestId> = all
                    .into_iter()
                    .filter(|(k, n)| k == binary && n.starts_with(prefix.as_str()))
                    .collect();
                if tests.is_empty() {
                    return Err(format!("No tests start with `{prefix}`."));
                }
                let mut args = b.cargo_args(None);
                args.extend(["--".into(), prefix.clone()]);
                Ok((args, tests))
            }
            Scope::Test((key, name)) => {
                let b = self.binary(key).ok_or(
                    "That test isn't in a known test target (doc tests run with Run All).",
                )?;
                Ok((
                    b.cargo_args(Some(name)),
                    HashSet::from([(key.clone(), name.clone())]),
                ))
            }
            Scope::Failed => {
                let failed: Vec<TestId> = all
                    .into_iter()
                    .filter(|id| self.state(id) == State::Failed && !id.0.is_empty())
                    .collect();
                if failed.is_empty() {
                    return Err("No tests have failed.".into());
                }
                // libtest takes several exact names; cargo runs every harness with them.
                let mut args = vec!["--no-fail-fast".into(), "--".into(), "--exact".into()];
                let names: std::collections::BTreeSet<&String> =
                    failed.iter().map(|(_, n)| n).collect();
                args.extend(names.into_iter().cloned());
                Ok((args, failed.into_iter().collect()))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotRun,
    Running,
    Passed,
    Failed,
    Ignored,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            State::NotRun => "not_run",
            State::Running => "running",
            State::Passed => "passed",
            State::Failed => "failed",
            State::Ignored => "ignored",
        }
    }

    fn icon(self, ui: &egui::Ui) -> egui::Image<'static> {
        let green = egui::Color32::from_rgb(0x4e, 0xc9, 0x6b);
        match self {
            State::NotRun => Icon::CircleDashed.colored(ui.visuals().weak_text_color()),
            State::Running => Icon::LoaderCircle.colored(ui.visuals().warn_fg_color),
            State::Passed => Icon::CircleCheck.colored(green),
            State::Failed => Icon::CircleX.colored(ui.visuals().error_fg_color),
            State::Ignored => Icon::CircleSlash.colored(ui.visuals().weak_text_color()),
        }
    }

    /// The state of a group: failed beats running beats not run beats passed.
    fn combine(states: impl Iterator<Item = State>) -> State {
        let mut out = None;
        for s in states {
            let rank = |s: State| match s {
                State::Failed => 4,
                State::Running => 3,
                State::NotRun => 2,
                State::Passed => 1,
                State::Ignored => 0,
            };
            if out.is_none_or(|o| rank(s) > rank(o)) {
                out = Some(s);
            }
        }
        out.unwrap_or(State::NotRun)
    }
}

/// Starts a run. Used by the pane's buttons and by `test.run`.
pub fn run(s: &mut AppState, scope: Scope) -> Result<Value, CommandError> {
    let (args, tests) = s.build.tests.plan(&scope).map_err(err)?;
    let n = tests.len();
    crate::ide::start_build_with(s, Kind::Test, &args)?;
    s.build.tests.begin(tests);
    s.layout_requests.push(LayoutRequest::ShowPane(Pane::Tests));
    Ok(json!({
        "started": format!("cargo test {}", args.join(" ")),
        "tests": n,
        "hint": "Call test.results when build.status says it's done."
    }))
}

fn err(msg: impl Into<String>) -> CommandError {
    CommandError::Failed(msg.into())
}

fn root(s: &AppState) -> Result<PathBuf, CommandError> {
    s.project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| err("No project is open."))
}

fn test_json(s: &AppState, id: &TestId, with_output: bool) -> Value {
    let t = &s.build.tests;
    let mut v = json!({ "name": id.1, "state": t.state(id).name(), "harness": id.0 });
    if let Some(b) = t.binary(&id.0) {
        v["target"] = json!(format!("{} ({})", b.target, b.kind));
        v["package"] = json!(b.package);
    } else if id.0.is_empty() {
        v["target"] = json!("doc tests");
    }
    if with_output && let Some(o) = t.output.get(id) {
        v["output"] = json!(o);
    }
    v
}

pub fn register(bus: &mut CommandBus<AppState>) {
    type Handler = fn(&mut AppState, &perox_core::Invocation) -> Result<Value, CommandError>;
    let commands: Vec<(CommandSpec, Handler)> = vec![
        (
            CommandSpec::new("view.tests", "Test Explorer", Access::Read).ui_only(),
            |s, _| {
                if !s.build.tests.discovered && !s.build.tests.is_discovering() {
                    let r = root(s)?;
                    s.build.tests.discover(r, &s.ctx);
                }
                s.layout_requests.push(LayoutRequest::ShowPane(Pane::Tests));
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("test.discover", "Find Tests", Access::Execute)
                .description("Builds the tests (`cargo test --no-run`) and lists every test in the project. Runs in the background: test.list reports `discovering` until it's done."),
            |s, _| {
                let r = root(s)?;
                s.build.tests.discover(r, &s.ctx);
                Ok(json!({ "discovering": true }))
            },
        ),
        (
            CommandSpec::new("test.list", "List Tests", Access::Read)
                .description("Every known test with its target and state (not_run, running, passed, failed, ignored), plus counts. `filter` keeps names containing it.")
                .params(json!({ "type": "object", "properties": { "filter": { "type": "string" } } })),
            |s, inv| {
                let filter = inv.args.get("filter").and_then(Value::as_str).unwrap_or("").to_owned();
                let t = &s.build.tests;
                let (passed, failed, ignored, not_run) = t.counts();
                let tests: Vec<Value> = t
                    .all()
                    .iter()
                    .filter(|id| id.1.contains(&filter))
                    .map(|id| test_json(s, id, false))
                    .collect();
                Ok(json!({
                    "discovering": t.is_discovering(),
                    "running": t.is_running(),
                    "error": t.error,
                    "counts": { "passed": passed, "failed": failed, "ignored": ignored, "not_run": not_run },
                    "tests": tests,
                }))
            },
        ),
        (
            CommandSpec::new("test.run", "Run Tests", Access::Execute)
                .description("Runs tests through cargo: all of them (no arguments), the failed ones (`failed: true`), one test (`name`, exact), or every test whose name starts with `prefix` (a module path). `target` narrows to one test target by name when names repeat.")
                .params(json!({ "type": "object", "properties": {
                    "name": { "type": "string" }, "prefix": { "type": "string" },
                    "target": { "type": "string" }, "harness": { "type": "string", "description": "A test target's harness id, as test.list gives it." },
                    "failed": { "type": "boolean" }
                } })),
            |s, inv| {
                let scope = scope_from(s, inv)?;
                run(s, scope)
            },
        ),
        (
            CommandSpec::new("test.results", "Test Results", Access::Read)
                .description("Results of the tests that have run, with the captured output of failures. `failed_only` (default true) leaves out the rest.")
                .params(json!({ "type": "object", "properties": { "failed_only": { "type": "boolean" } } })),
            |s, inv| {
                let failed_only = inv.bool_arg("failed_only")?.unwrap_or(true);
                let t = &s.build.tests;
                let (passed, failed, ignored, not_run) = t.counts();
                let tests: Vec<Value> = t
                    .all()
                    .iter()
                    .filter(|id| match t.state(id) {
                        State::Failed => true,
                        State::NotRun => false,
                        _ => !failed_only,
                    })
                    .map(|id| test_json(s, id, true))
                    .collect();
                Ok(json!({
                    "running": t.is_running(),
                    "counts": { "passed": passed, "failed": failed, "ignored": ignored, "not_run": not_run },
                    "tests": tests,
                }))
            },
        ),
        (
            CommandSpec::new("test.go_to", "Go to Test", Access::Read)
                .description("Opens the source of a test (by full name) at its function.")
                .params(json!({ "type": "object", "required": ["name"], "properties": { "name": { "type": "string" }, "target": { "type": "string" }, "harness": { "type": "string" } } })),
            |s, inv| {
                let scope = scope_from(s, inv)?;
                let Scope::Test(id) = scope else {
                    return Err(err("Give the test's full `name`."));
                };
                let (path, line) = locate(&s.build.tests, &id).ok_or_else(|| err("Couldn't find that test's source."))?;
                crate::ide::open_file(s, &path, Some(line), None)?;
                s.build.tests.selected = Some(id);
                Ok(json!({ "path": path, "line": line }))
            },
        ),
    ];
    for (spec, h) in commands {
        bus.register(spec, h)
            .expect("test command ids are valid and unique");
    }
}

fn scope_from(s: &AppState, inv: &perox_core::Invocation) -> Result<Scope, CommandError> {
    let t = &s.build.tests;
    let target = inv.args.get("target").and_then(Value::as_str);
    let harness = inv.args.get("harness").and_then(Value::as_str);
    let pick = |pred: &dyn Fn(&Binary) -> bool| -> Result<String, CommandError> {
        let found: Vec<&Binary> = t
            .binaries
            .iter()
            .filter(|b| {
                pred(b)
                    && target.is_none_or(|tn| b.target == tn)
                    && harness.is_none_or(|h| b.key() == h)
            })
            .collect();
        match found.as_slice() {
            [b] => Ok(b.key()),
            [] => Err(err(if t.binaries.is_empty() {
                "No tests are known yet: run test.discover first."
            } else {
                "No test target matches."
            })),
            _ => Err(err("Several test targets match: pass `target`.")),
        }
    };
    if inv.bool_arg("failed")?.unwrap_or(false) {
        return Ok(Scope::Failed);
    }
    if let Some(name) = inv.args.get("name").and_then(Value::as_str) {
        let key = pick(&|b| b.tests.iter().any(|n| n == name))?;
        return Ok(Scope::Test((key, name.to_owned())));
    }
    if let Some(prefix) = inv.args.get("prefix").and_then(Value::as_str) {
        let key = pick(&|b| b.tests.iter().any(|n| n.starts_with(prefix)))?;
        return Ok(Scope::Prefix {
            binary: key,
            prefix: prefix.to_owned(),
        });
    }
    if target.is_some() || harness.is_some() {
        return Ok(Scope::Binary(pick(&|_| true)?));
    }
    Ok(Scope::All)
}

/// Where a test's function is.
pub fn locate(t: &Tests, id: &TestId) -> Option<(PathBuf, usize)> {
    let b = t.binary(&id.0)?;
    perox_build::testing::locate(&b.src, &id.1)
}

/// Module tree of one harness's tests, for drawing.
#[derive(Default)]
struct Node {
    modules: BTreeMap<String, Node>,
    tests: Vec<String>,
}

impl Node {
    fn insert(&mut self, full: &str) {
        let mut node = self;
        let mut parts: Vec<&str> = full.split("::").collect();
        parts.pop();
        for p in parts {
            node = node.modules.entry(p.to_owned()).or_default();
        }
        node.tests.push(full.to_owned());
    }
}

/// What the pane asks for; the app turns these into commands.
pub enum PaneAction {
    Run(Scope),
    Discover,
    Stop,
    Open(TestId),
    /// Run one test under the debugger.
    Debug(TestId),
    /// A file and 1-based line (where a failed test panicked).
    OpenAt(PathBuf, usize),
    Select(TestId),
}

/// Draws the Test Explorer.
pub fn show(ui: &mut egui::Ui, t: &mut Tests, project_open: bool) -> Vec<PaneAction> {
    let mut out = Vec::new();
    if !project_open {
        ui.weak("No project open.");
        return out;
    }
    let running = t.is_running();
    ui.horizontal(|ui| {
        let run_all = egui::Button::image_and_text(Icon::Play.image(ui), "Run All");
        if ui
            .add_enabled(!running, run_all)
            .on_hover_text("cargo test")
            .clicked()
        {
            out.push(PaneAction::Run(Scope::All));
        }
        let (_, failed, _, _) = t.counts();
        let rerun = egui::Button::image_and_text(Icon::RotateCcw.image(ui), "Run Failed");
        if ui.add_enabled(!running && failed > 0, rerun).clicked() {
            out.push(PaneAction::Run(Scope::Failed));
        }
        if running {
            let stop = egui::Button::image_and_text(Icon::Square.image(ui), "Stop");
            if ui.add(stop).clicked() {
                out.push(PaneAction::Stop);
            }
        }
        let busy = t.is_discovering();
        let refresh = egui::Button::image(Icon::RefreshCw.image(ui));
        if ui
            .add_enabled(!busy && !running, refresh)
            .on_hover_text("Find tests again (builds them)")
            .clicked()
        {
            out.push(PaneAction::Discover);
        }
        if busy {
            ui.spinner();
            ui.weak("Finding tests…");
        }
        ui.add(
            egui::TextEdit::singleline(&mut t.filter)
                .hint_text("Filter")
                .desired_width(160.0),
        );
    });
    let (passed, failed, ignored, not_run) = t.counts();
    ui.horizontal(|ui| {
        let green = egui::Color32::from_rgb(0x4e, 0xc9, 0x6b);
        ui.add(State::Passed.icon(ui));
        ui.colored_label(green, format!("{passed} passed"));
        ui.add(State::Failed.icon(ui));
        ui.colored_label(
            if failed > 0 {
                ui.visuals().error_fg_color
            } else {
                ui.visuals().weak_text_color()
            },
            format!("{failed} failed"),
        );
        if ignored > 0 {
            ui.weak(format!("{ignored} ignored"));
        }
        ui.weak(format!("{not_run} not run"));
    });
    if let Some(e) = &t.error {
        ui.colored_label(ui.visuals().error_fg_color, e);
    }
    ui.separator();
    if t.binaries.is_empty() && t.results.is_empty() {
        if !t.is_discovering() {
            ui.weak(if t.discovered {
                "No tests found."
            } else {
                "Press Refresh to find tests."
            });
        }
        return out;
    }

    // The selected failure's output goes underneath the tree.
    let detail = t
        .selected
        .as_ref()
        .and_then(|id| t.output.get(id).map(|o| (id.1.clone(), o.clone())));
    let filter = t.filter.to_lowercase();
    // Groups: each harness, then results from unknown harnesses (doc tests).
    let mut groups: Vec<(String, String, Node)> = Vec::new();
    for b in &t.binaries {
        let mut node = Node::default();
        for name in b
            .tests
            .iter()
            .filter(|n| n.to_lowercase().contains(&filter))
        {
            node.insert(name);
        }
        let label = if b.kind == "lib" {
            format!("{} (lib)", b.package)
        } else {
            format!("{} › {} ({})", b.package, b.target, b.kind)
        };
        groups.push((b.key(), label, node));
    }
    let known: HashSet<String> = t.binaries.iter().map(Binary::key).collect();
    let mut extra: BTreeMap<String, Node> = BTreeMap::new();
    for (k, n) in t.results.keys() {
        if !known.contains(k) && n.to_lowercase().contains(&filter) {
            // Doc test names are paths; keep them whole.
            extra.entry(k.clone()).or_default().tests.push(n.clone());
        }
    }
    for (k, mut node) in extra {
        node.tests.sort();
        let label = if k.is_empty() {
            "Doc tests".to_owned()
        } else {
            k.clone()
        };
        groups.push((k, label, node));
    }

    let draw_tree = |ui: &mut egui::Ui, out: &mut Vec<PaneAction>| {
        egui::ScrollArea::vertical()
            .id_salt("tests_tree")
            .auto_shrink(false)
            .show(ui, |ui| {
                for (key, label, node) in &groups {
                    if node.tests.is_empty() && node.modules.is_empty() {
                        continue;
                    }
                    let state = group_state(t, key, node);
                    let id = ui.make_persistent_id(("tests", key));
                    egui::collapsing_header::CollapsingState::load_with_default_open(
                        ui.ctx(),
                        id,
                        true,
                    )
                    .show_header(ui, |ui| {
                        ui.add(state.icon(ui));
                        ui.strong(label);
                        if !key.is_empty() && run_button(ui, running, "Run this target") {
                            out.push(PaneAction::Run(Scope::Binary(key.clone())));
                        }
                    })
                    .body(|ui| draw_node(ui, t, key, node, "", running, out));
                }
            });
    };
    let Some((name, text)) = detail else {
        draw_tree(ui, &mut out);
        return out;
    };
    let draw_detail = |ui: &mut egui::Ui, out: &mut Vec<PaneAction>| {
        ui.horizontal(|ui| {
            ui.add(State::Failed.icon(ui));
            ui.strong(&name);
            if let Some((file, line, col)) = panic_location(&text) {
                let link = ui.link(format!("{file}:{line}:{col}"));
                if link.on_hover_text("Go to where it panicked").clicked() {
                    out.push(PaneAction::OpenAt(PathBuf::from(file), line));
                }
            }
        });
        egui::ScrollArea::both()
            .id_salt("tests_output")
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(text.trim_matches('\n').trim_end()).monospace())
                        .extend(),
                );
            });
    };
    // Bottom panels are wide and short: output beside the tree when there's room.
    if ui.available_width() > 640.0 {
        let mut left = Vec::new();
        let mut right = Vec::new();
        ui.columns(2, |cols| {
            draw_tree(&mut cols[0], &mut left);
            draw_detail(&mut cols[1], &mut right);
        });
        out.extend(left);
        out.extend(right);
    } else {
        let h = ui.available_height() * 0.5;
        ui.allocate_ui(egui::vec2(ui.available_width(), h), |ui| {
            draw_tree(ui, &mut out)
        });
        ui.separator();
        draw_detail(ui, &mut out);
    }
    out
}

/// `panicked at src/lib.rs:24:9:` in a test's output: file, line and column.
pub fn panic_location(output: &str) -> Option<(&str, usize, usize)> {
    let at = output.find("panicked at ")? + "panicked at ".len();
    let rest = output[at..].lines().next()?.trim_end_matches(':');
    let (rest, col) = rest.rsplit_once(':')?;
    let (file, line) = rest.rsplit_once(':')?;
    Some((file, line.parse().ok()?, col.parse().ok()?))
}

fn run_button(ui: &mut egui::Ui, running: bool, tip: &str) -> bool {
    ui.add_enabled(
        !running,
        egui::Button::image(Icon::Play.image(ui))
            .frame(false)
            .small(),
    )
    .on_hover_text(tip)
    .clicked()
}

fn group_state(t: &Tests, key: &str, node: &Node) -> State {
    let mut states: Vec<State> = node
        .tests
        .iter()
        .map(|n| t.state(&(key.to_owned(), n.clone())))
        .collect();
    states.extend(node.modules.values().map(|m| group_state(t, key, m)));
    State::combine(states.into_iter())
}

fn draw_node(
    ui: &mut egui::Ui,
    t: &Tests,
    key: &str,
    node: &Node,
    path: &str,
    running: bool,
    out: &mut Vec<PaneAction>,
) {
    for (name, child) in &node.modules {
        // A module holding just one module shows as one row: `shapes::circle::tests`.
        let mut name = name.clone();
        let mut child = child;
        while child.tests.is_empty() && child.modules.len() == 1 {
            let (n, c) = child.modules.iter().next().expect("one module");
            name = format!("{name}::{n}");
            child = c;
        }
        let name = &name;
        let full = format!("{path}{name}::");
        let state = group_state(t, key, child);
        let id = ui.make_persistent_id(("tests", key, &full));
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.add(state.icon(ui));
                ui.label(name);
                if !key.is_empty() && run_button(ui, running, "Run the tests in this module") {
                    out.push(PaneAction::Run(Scope::Prefix {
                        binary: key.to_owned(),
                        prefix: full.clone(),
                    }));
                }
            })
            .body(|ui| draw_node(ui, t, key, child, &full, running, out));
    }
    for full in &node.tests {
        let id: TestId = (key.to_owned(), full.clone());
        let state = t.state(&id);
        let short = full.strip_prefix(path).unwrap_or(full);
        ui.horizontal(|ui| {
            ui.add(state.icon(ui));
            let selected = t.selected.as_ref() == Some(&id);
            let r = ui.add(egui::Button::selectable(selected, short));
            if r.clicked() {
                out.push(PaneAction::Select(id.clone()));
            }
            if r.double_clicked() {
                out.push(PaneAction::Open(id.clone()));
            }
            r.on_hover_text("Double-click to go to the test");
            if !key.is_empty() && run_button(ui, running, "Run this test") {
                out.push(PaneAction::Run(Scope::Test(id.clone())));
            }
            if !key.is_empty()
                && ui
                    .add_enabled(
                        !running,
                        egui::Button::image(Icon::Bug.image(ui))
                            .frame(false)
                            .small(),
                    )
                    .on_hover_text("Debug this test")
                    .clicked()
            {
                out.push(PaneAction::Debug(id.clone()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(key: &str, tests: &[&str]) -> Binary {
        Binary {
            package: "p".into(),
            target: "p".into(),
            kind: "lib".into(),
            src: PathBuf::from("/p/src/lib.rs"),
            exe: PathBuf::from(format!("/p/target/debug/deps/{key}")),
            tests: tests.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_run_updates_states() {
        let mut t = Tests {
            binaries: vec![bin("p-1", &["a::one", "a::two", "b"])],
            ..Default::default()
        };
        let (args, set) = t.plan(&Scope::All).unwrap();
        assert_eq!(args, ["--no-fail-fast"]);
        assert_eq!(set.len(), 3);
        t.begin(set);
        assert_eq!(t.state(&("p-1".into(), "b".into())), State::Running);
        for l in [
            "     Running unittests src/lib.rs (target/debug/deps/p-1)",
            "test a::one ... ok",
            "test a::two ... FAILED",
            "---- a::two stdout ----",
            "boom",
            "",
            "failures:",
            "   Doc-tests p",
            "test src/lib.rs - f (line 1) ... ok",
        ] {
            t.feed(l);
        }
        t.finish();
        assert_eq!(t.state(&("p-1".into(), "a::one".into())), State::Passed);
        assert_eq!(t.state(&("p-1".into(), "a::two".into())), State::Failed);
        assert_eq!(t.state(&("p-1".into(), "b".into())), State::NotRun);
        assert_eq!(
            t.state(&("".into(), "src/lib.rs - f (line 1)".into())),
            State::Passed
        );
        assert_eq!(
            t.output[&("p-1".to_owned(), "a::two".to_owned())].trim(),
            "boom"
        );
        assert_eq!(t.counts(), (2, 1, 0, 1));

        let (args, set) = t.plan(&Scope::Failed).unwrap();
        assert_eq!(args, ["--no-fail-fast", "--", "--exact", "a::two"]);
        assert_eq!(set.len(), 1);
        let (args, set) = t
            .plan(&Scope::Prefix {
                binary: "p-1".into(),
                prefix: "a::".into(),
            })
            .unwrap();
        assert_eq!(args, ["-p", "p", "--lib", "--", "a::"]);
        assert_eq!(set.len(), 2);
        assert!(t.plan(&Scope::Test(("".into(), "x".into()))).is_err());
    }

    #[test]
    fn finds_the_panic_location() {
        let out = "\nthread 'a' (5155863) panicked at src/lib.rs:24:9:\nassertion failed\n";
        assert_eq!(panic_location(out), Some(("src/lib.rs", 24, 9)));
        assert_eq!(panic_location("no panic here"), None);
    }

    #[test]
    fn group_state_prefers_failures() {
        assert_eq!(
            State::combine([State::Passed, State::Failed, State::NotRun].into_iter()),
            State::Failed
        );
        assert_eq!(
            State::combine([State::Passed, State::Ignored].into_iter()),
            State::Passed
        );
    }
}
