//! IDE commands: projects, files, editing, building, problems. Menus, shortcuts, the
//! palette and agents all use these. Agents' file access is limited to the project.
//! Lines and columns in arguments and results are 1-based, as people count them.

use std::path::{Path, PathBuf};

use perox_build::{Job, Kind};
use perox_core::{Access, CommandBus, CommandError, CommandSpec, Invocation, Origin};
use perox_editor::{Change, Pos, Request};
use perox_project::Project;
use serde_json::{Value, json};

use crate::commands::AppState;
use crate::docs::DocId;

/// Something only the layout can do; queued by commands, done by the app after them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutRequest {
    ShowDoc(DocId),
    CloseDoc(DocId),
    ShowOutput,
    ShowReferences,
    ShowTerminal,
    ShowSourceControl,
    ShowPane(crate::layout::Pane),
}

/// The current cargo job and what it printed.
/// From the packaging thread: `Ok(log line)`, then `Err(result)` at the end.
pub type PackMsg = Result<String, Result<Vec<PathBuf>, String>>;

#[derive(Debug, Default)]
pub struct BuildState {
    pub job: Option<Job>,
    pub output: Vec<String>,
    pub diagnostics: Vec<perox_build::Diagnostic>,
    /// The last finished job and whether it succeeded.
    pub last: Option<(Kind, bool)>,
    /// The Test Explorer; it reads the output of test runs.
    pub tests: crate::testing::Tests,
    /// Executables the current (or last) job built.
    pub artifacts: Vec<perox_build::Artifact>,
    /// Package these once the release build that's running succeeds (Build › Package App).
    pub package_after: Option<(PathBuf, Vec<perox_package::Format>)>,
    /// Packaging running on a thread: its log lines, then the result.
    pub packaging: Option<std::sync::mpsc::Receiver<PackMsg>>,
    /// What the last packaging made, or why it failed.
    pub packaged: Option<Result<Vec<PathBuf>, String>>,
}

impl BuildState {
    pub fn running(&self) -> Option<Kind> {
        self.job.as_ref().map(|j| j.kind)
    }

    /// Collects packaging's log; returns the result when it finished this frame.
    pub fn poll_packaging(&mut self) -> Option<Result<Vec<PathBuf>, String>> {
        let rx = self.packaging.as_ref()?;
        let mut done = None;
        while let Ok(m) = rx.try_recv() {
            match m {
                Ok(line) => self.output.push(line),
                Err(result) => done = Some(result),
            }
        }
        let result = done?;
        self.packaging = None;
        match &result {
            Ok(files) => self
                .output
                .push(format!("── packaged: {} file(s) ──", files.len())),
            Err(e) => {
                self.output.push(e.clone());
                self.output.push("── packaging failed ──".into());
            }
        }
        self.packaged = Some(result.clone());
        Some(result)
    }

    /// Collects new output; returns `Some(success)` when the job finished this frame.
    pub fn poll(&mut self) -> Option<(Kind, bool)> {
        let job = self.job.as_mut()?;
        let kind = job.kind;
        let mut finished = None;
        for e in job.poll() {
            match e {
                perox_build::Event::Line(l) => {
                    if kind == Kind::Test {
                        self.tests.feed(&l);
                    }
                    if self.output.len() >= 20_000 {
                        self.output.drain(..5_000);
                    }
                    self.output.push(l);
                }
                perox_build::Event::Diagnostic(d) => self.diagnostics.push(d),
                perox_build::Event::Artifact(a) => self.artifacts.push(a),
                perox_build::Event::Finished { success } => finished = Some(success),
            }
        }
        let success = finished?;
        self.job = None;
        if kind == Kind::Test {
            self.tests.finish();
        }
        self.last = Some((kind, success));
        self.output.push(String::new());
        self.output.push(format!(
            "── cargo {} {} ──",
            kind.verb(),
            if success { "finished" } else { "failed" }
        ));
        Some((kind, success))
    }
}

fn err(msg: impl Into<String>) -> CommandError {
    CommandError::Failed(msg.into())
}

fn opt_usize(inv: &Invocation, name: &str) -> Option<usize> {
    inv.args
        .get(name)
        .and_then(Value::as_u64)
        .map(|n| n as usize)
}

/// Resolves a path argument against the project. Agents may only reach inside it.
pub fn resolve(s: &AppState, inv: &Invocation, name: &str) -> Result<PathBuf, CommandError> {
    let raw = inv.str_arg(name)?;
    let root = s.project.as_ref().map(|p| p.root.clone());
    let path = match (&root, Path::new(raw).is_absolute()) {
        (_, true) => PathBuf::from(raw),
        (Some(r), false) => r.join(raw),
        (None, false) => {
            return Err(err(
                "No project is open; use an absolute path or open a project first.",
            ));
        }
    };
    if matches!(inv.origin, Origin::Agent { .. }) {
        let Some(root) = root else {
            return Err(err(
                "No project is open. Agents can only use files inside the open project.",
            ));
        };
        let normal = normalize(&path);
        if !normal.starts_with(normalize(&root)) {
            return Err(err(format!(
                "{} is outside the project ({}).",
                path.display(),
                root.display()
            )));
        }
    }
    Ok(path)
}

/// Removes `.` and `..` without touching the disk (the file may not exist yet).
/// Resolves `.` and `..` without touching the disk.
pub fn normalize_path(p: &Path) -> PathBuf {
    normalize(p)
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn rel(s: &AppState, p: &Path) -> String {
    s.project
        .as_ref()
        .and_then(|pr| p.strip_prefix(&pr.root).ok())
        .unwrap_or(p)
        .display()
        .to_string()
}

/// Opens (or focuses) a file and optionally moves the cursor to a 1-based line/column.
pub fn open_file(
    s: &mut AppState,
    path: &Path,
    line: Option<usize>,
    col: Option<usize>,
) -> Result<DocId, CommandError> {
    let (id, new) = s.docs.open(path).map_err(err)?;
    if new {
        s.lsp.did_open(id, &s.docs);
    }
    if let (Some(line), Some(doc)) = (line, s.docs.get_mut(id)) {
        doc.editor
            .goto(line.saturating_sub(1), col.unwrap_or(1).saturating_sub(1));
    }
    if let Some(doc) = s.docs.get_mut(id) {
        doc.editor.focus_requested = true;
    }
    s.active_doc = Some(id);
    s.layout_requests.push(LayoutRequest::ShowDoc(id));
    Ok(id)
}

pub fn open_project(s: &mut AppState, path: &Path) -> Result<Value, CommandError> {
    let cargo = perox_build::tools::cargo()
        .ok_or_else(|| err("cargo isn't installed (Toolchain Set missing)."))?;
    let project = Project::open(path, &cargo).map_err(err)?;
    s.tree = Some(project.file_tree(20_000));
    s.files = project.files(50_000);
    s.lsp.start(&project.root);
    s.build.diagnostics.clear();
    s.build.tests.reset();
    let saved = s
        .settings
        .sessions
        .get(&project.root)
        .map(|x| x.breakpoints.clone())
        .unwrap_or_default();
    s.debug.reset(saved);
    let info = project_info(&project);
    s.status = format!("Opened {}", project.name);
    s.save_session(); // the project being left
    s.settings.remember_project(&project.root);
    s.watcher = crate::watcher::Watcher::new(&project.root, s.ctx.clone());
    s.git = crate::git::Git::open(&project.root);
    s.git.refresh(&s.ctx);
    let session = s
        .settings
        .sessions
        .get(&project.root)
        .cloned()
        .unwrap_or_default();
    s.project = Some(project);
    for path in session.open.iter().filter(|p| p.is_file()) {
        let _ = open_file(s, path, None, None);
    }
    if let Some(active) = session.active.filter(|p| p.is_file()) {
        let _ = open_file(s, &active, None, None);
    }
    Ok(info)
}

fn project_info(p: &Project) -> Value {
    json!({
        "root": p.root,
        "name": p.name,
        "packages": p.packages.iter().map(|pk| json!({
            "name": pk.name,
            "targets": pk.targets.iter().map(|t| json!({ "name": t.name, "kind": t.kind, "src": t.src_path })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

fn start_build(s: &mut AppState, kind: Kind) -> Result<Value, CommandError> {
    start_build_with(s, kind, &[])
}

/// Starts `cargo <kind> <args…>` after saving every file.
pub fn start_build_with(
    s: &mut AppState,
    kind: Kind,
    args: &[String],
) -> Result<Value, CommandError> {
    start_build_env(s, kind, args, &[])
}

/// [`start_build_with`], with environment variables for cargo and the program.
pub fn start_build_env(
    s: &mut AppState,
    kind: Kind,
    args: &[String],
    env: &[(String, String)],
) -> Result<Value, CommandError> {
    let root = s
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| err("No project is open."))?;
    let cargo = perox_build::tools::cargo().ok_or_else(|| err("cargo isn't installed."))?;
    // Save everything first, as people expect.
    let dirty: Vec<DocId> = s
        .docs
        .list
        .iter()
        .filter(|d| d.is_dirty())
        .map(|d| d.id)
        .collect();
    for id in dirty {
        s.docs.save(id).map_err(err)?;
        s.lsp.did_save(id, &s.docs);
        s.git.refresh(&s.ctx);
    }
    if let Some(job) = &mut s.build.job {
        job.stop();
    }
    s.build.tests.finish(); // a test run that was going on has ended
    let ctx = s.ctx.clone();
    let job = Job::start_env(&cargo, &root, kind, args, env, move || {
        ctx.request_repaint()
    })
    .map_err(|e| err(e.to_string()))?;
    s.build.job = Some(job);
    s.build.output.clear();
    s.build.diagnostics.clear();
    s.build.artifacts.clear();
    s.build.output.push(format!("── cargo {} ──", kind.verb()));
    s.layout_requests.push(LayoutRequest::ShowOutput);
    s.status = format!("cargo {}…", kind.verb());
    Ok(
        json!({ "started": format!("cargo {}", kind.verb()), "hint": "Call build.output (or problems.list) for results; build.status says when it's done." }),
    )
}

/// Build › Package App: a release build, then the packages (see [`package_built`]).
fn start_package(
    s: &mut AppState,
    app: Option<PathBuf>,
    formats: Vec<perox_package::Format>,
) -> Result<Value, CommandError> {
    let root = s
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| err("No project is open."))?;
    let has_package = |dir: &Path| {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|t| t.lines().any(|l| l.trim() == "[package]"))
    };
    // The app: the one named, else the project, else (in a workspace) the package of
    // the file being edited.
    let app = match app {
        Some(a) => a,
        None if has_package(&root) => root.clone(),
        None => s
            .active_doc
            .and_then(|id| s.docs.get(id))
            .and_then(|d| {
                d.path
                    .ancestors()
                    .take_while(|a| a.starts_with(&root))
                    .find(|a| has_package(a))
                    .map(Path::to_path_buf)
            })
            .ok_or_else(|| {
                err("This is a workspace: open a file of the app to package (or pass `path`, its folder).")
            })?,
    };
    // Fail early on an app that can't be packaged.
    let info = perox_package::AppInfo::load(&app, &root.join("target")).map_err(err)?;
    let mut args: Vec<String> = vec!["--release".into()];
    if app != root {
        args.extend(["-p".into(), info.name.clone()]);
    }
    let started = start_build_with(s, Kind::Build, &args)?;
    s.build.package_after = Some((app, formats.clone()));
    s.build.packaged = None;
    let names: Vec<&str> = formats.iter().map(|f| f.name()).collect();
    Ok(json!({
        "started": started["started"],
        "app": info.display_name,
        "formats": names,
        "hint": "A release build, then packaging. build.status shows `packaging` while it runs and `packaged` (the files, in target/peroxide/package) when done."
    }))
}

/// Called when a build finishes: if it was Package App's release build, packages it.
pub fn package_built(s: &mut AppState, ok: bool) {
    let Some((app, formats)) = s.build.package_after.take() else {
        return;
    };
    if !ok {
        s.build.packaged = Some(Err(
            "The release build failed, so nothing was packaged.".into()
        ));
        return;
    }
    let mut info = match perox_package::AppInfo::load(&app, &app.join("target")) {
        Ok(i) => i,
        Err(e) => {
            s.build.output.push(e.clone());
            s.build.packaged = Some(Err(e));
            return;
        }
    };
    // The build says where it put the executable (a workspace's target folder, say).
    if let Some(a) = s
        .build
        .artifacts
        .iter()
        .find(|a| a.kind == "bin" && !a.test && a.target == info.bin)
    {
        info.binary = a.executable.clone();
    }
    let root = s.project.as_ref().map_or(app.clone(), |p| p.root.clone());
    let out = root.join("target").join("peroxide").join("package");
    let (tx, rx) = std::sync::mpsc::channel();
    s.build.packaging = Some(rx);
    s.build.output.push(String::new());
    s.build.output.push(format!(
        "── packaging {} {} ──",
        info.display_name, info.version
    ));
    s.status = "Packaging…".into();
    let ctx = s.ctx.clone();
    std::thread::spawn(move || {
        let log_tx = tx.clone();
        let log_ctx = ctx.clone();
        let mut log = move |l: &str| {
            let _ = log_tx.send(Ok(l.to_owned()));
            log_ctx.request_repaint();
        };
        let result = perox_package::package(&info, &out, &formats, &mut log);
        let _ = tx.send(Err(result));
        ctx.request_repaint();
    });
}

type Handler = fn(&mut AppState, &Invocation) -> Result<Value, CommandError>;

fn path_schema(extra: Value) -> Value {
    let mut props = json!({ "path": { "type": "string", "description": "File path, relative to the project root or absolute." } });
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    json!({ "type": "object", "required": ["path"], "properties": props })
}

pub fn register(bus: &mut CommandBus<AppState>) {
    let line_col = json!({
        "line": { "type": "integer", "minimum": 1 },
        "column": { "type": "integer", "minimum": 1 }
    });
    let build = |id: &'static str, title: &'static str, desc: &'static str| {
        CommandSpec::new(id, title, Access::Execute).description(desc)
    };
    let commands: Vec<(CommandSpec, Handler)> = vec![
        // ── form designer ──
        (
            CommandSpec::new("designer.open_handler", "Go to Event Handler", Access::Edit)
                .description("Shows a callback handler in the code-behind (the `impl <Component>Handlers for …` block), adding an empty method if it's missing.")
                .params(json!({
                    "type": "object",
                    "properties": {
                        "form": { "type": "string", "description": "Component, e.g. MainWindow" },
                        "method": { "type": "string" },
                        "params": { "type": "array", "items": { "type": "array", "items": { "type": "string" } }, "description": "[[name, Rust type], …] for a new method" }
                    },
                    "required": ["form", "method"]
                })),
            |s, inv| {
                let form = inv.str_arg("form")?.to_owned();
                let method = inv.str_arg("method")?.to_owned();
                let params: Vec<(String, String)> = inv.args["params"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| Some((p[0].as_str()?.to_owned(), p[1].as_str()?.to_owned())))
                    .collect();
                open_handler(s, &form, &method, &params)
            },
        ),
        (
            CommandSpec::new("designer.screenshot", "Form Screenshot", Access::Execute)
                .description("Renders a .slint component (the file's main window by default) to a PNG, exactly as Slint draws it, at its natural size or `width`×`height` points. `style` (fluent, cupertino, material, cosmic; default: the app's, else this OS's), `dark` (default: the app's scheme, else light), `scale` (pixels per point, default 2). Saved under target/peroxide/screenshots/ unless `out` (inside the project) is given; returns the path. Uses unsaved edits if the file is open. Tables bound to queries show the design database's rows.")
                .params(path_schema(json!({
                    "component": { "type": "string" }, "style": { "type": "string" },
                    "dark": { "type": "boolean" }, "scale": { "type": "number", "minimum": 0.5, "maximum": 4 },
                    "width": { "type": "number" }, "height": { "type": "number" },
                    "out": { "type": "string" },
                    "language": { "type": "string", "description": "Show it translated (a language from i18n.strings)" }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                if !crate::docs::is_form(&path) {
                    return Err(err("Not a .slint file."));
                }
                let text = match s.docs.list.iter().find(|d| d.path == path) {
                    Some(d) => d.editor.core.text(),
                    None => std::fs::read_to_string(&path).map_err(|e| err(e.to_string()))?,
                };
                let doc = perox_slint::parse(&text);
                let component = component_name(&doc, inv)?;
                let app = perox_slint::project::settings_for(path.parent().unwrap_or(std::path::Path::new(".")));
                let style = inv.args["style"]
                    .as_str()
                    .map(str::to_owned)
                    .or(app.style.clone())
                    .unwrap_or_else(|| perox_designer::preview::native_style().to_owned());
                if !perox_designer::preview::STYLES.contains(&style.as_str()) {
                    return Err(err(format!("`style` is one of {}", perox_designer::preview::STYLES.join(", "))));
                }
                let dark = inv.bool_arg("dark")?.unwrap_or(app.color_scheme.as_deref() == Some("dark"));
                let scale = inv.args["scale"].as_f64().unwrap_or(2.0).clamp(0.5, 4.0) as f32;
                let size = match (inv.args["width"].as_f64(), inv.args["height"].as_f64()) {
                    (Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some(eframe::egui::vec2(w as f32, h as f32)),
                    (None, None) => None,
                    _ => return Err(err("Give both `width` and `height`, or neither.")),
                };
                let language = inv.args["language"].as_str().map(|l| {
                    let (dir, domain) = perox_designer::translations::location(path.parent().unwrap_or(std::path::Path::new(".")));
                    (dir.join(perox_slint::i18n::DIR), domain, l.to_owned())
                });
                let design_data = perox_designer::data::design_database(path.parent().unwrap_or(std::path::Path::new(".")))
                    .map(|db| perox_designer::data::design_rows(&mut perox_designer::data::Cache::default(), &db, &text, &component))
                    .unwrap_or_default();
                let (w, h, rgba) = perox_designer::preview::render_component_in(&text, &path, &component, &style, dark, size, scale, language, design_data)
                    .map_err(|e| err(format!("The form doesn't compile:\n{e}")))?;
                let root = s.project.as_ref().map(|p| p.root.clone()).ok_or_else(|| err("No project is open."))?;
                let out = match inv.args["out"].as_str() {
                    Some(o) => {
                        let o = normalize(&root.join(o));
                        if !o.starts_with(&root) {
                            return Err(err("`out` must be inside the project."));
                        }
                        o
                    }
                    None => {
                        let stem = path.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
                        root.join("target").join("peroxide").join("screenshots")
                            .join(format!("{stem}-{component}-{style}{}.png", if dark { "-dark" } else { "" }))
                    }
                };
                if let Some(dir) = out.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| err(e.to_string()))?;
                }
                image::save_buffer(&out, &rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8)
                    .map_err(|e| err(format!("couldn't write the PNG: {e}")))?;
                Ok(json!({ "path": out, "component": component, "style": style, "width": w, "height": h }))
            },
        ),
        (
            CommandSpec::new("designer.tree", "Form Tree", Access::Read)
                .description("A .slint file's components, and the designed one as a tree: each element's kind, id, path (#0.1), properties (as .slint expressions) and callbacks. Also lists syntax errors and the preview's compile errors.")
                .params(path_schema(json!({ "component": { "type": "string", "description": "Default: the file's main window" } }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let c = doc.component(&comp).ok_or_else(|| err(format!("No component `{comp}`.")))?;
                let mut errors: Vec<String> = doc.errors.iter().map(|e| e.message.clone()).collect();
                if let Some(d) = s.docs.get(id).and_then(|d| d.designer.as_ref()) {
                    errors.extend(d.preview.diagnostics.iter().cloned());
                }
                let callbacks: Vec<String> = c.callbacks.iter().map(|cb| s_slice(&text, &cb.span)).collect();
                Ok(json!({
                    "components": doc.components.iter().map(|c| &c.name).collect::<Vec<_>>(),
                    "component": comp,
                    "callbacks": callbacks,
                    "root": form_json(&c.root, &mut Vec::new()),
                    "errors": errors,
                }))
            },
        ),
        (
            CommandSpec::new("designer.add_widget", "Add Element", Access::Edit)
                .description("Adds an element to a component (in the editor, unsaved), importing it from std-widgets.slint if needed. Kinds: VerticalBox HorizontalBox GridBox Rectangle GroupBox TabWidget Tab ScrollView Text Button LineEdit TextEdit CheckBox Switch ComboBox SpinBox Slider ProgressIndicator Spinner StandardListView StandardTableView Path Image TouchArea; Canvas (an Image the Rust code draws into with perox_ui::canvas: declares <id>_image, <id>_width/<id>_height and callback <id>_resized(), wired to redraw on resize); ToolBar (a HorizontalBox named toolbar, for buttons that run actions: see designer.use_action); menus: MenuBar (in a Window, goes first) Menu (title) MenuItem (title, shortcut: \"@keys(Control + O)\", event activated) MenuSeparator ContextMenuArea (its Menu goes first, then the content it covers); non-visual: Timer (interval: \"500ms\", running, event triggered; goes at the top of the root). Or any component (other std-widgets ones are imported too; the project's own and its libraries' are imported from their files). In a component, kind \"@children\" marks where the elements forms put inside it go. Property values are .slint expressions: \"\\\"Hello\\\"\", \"12px\", \"true\", \"#ff8800\", \"[\\\"a\\\", \\\"b\\\"]\".")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "parent": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "index": { "type": "integer", "minimum": 0, "description": "Child position; default: last" },
                    "kind": { "type": "string" },
                    "id": { "type": "string" },
                    "props": { "type": "object", "additionalProperties": { "type": "string" } }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let kind = inv.str_arg("kind")?;
                if kind == "@children" {
                    // Where a component shows what forms put inside it.
                    let parent = widget_path(&doc, &comp, inv.str_arg("parent")?)?;
                    let index = opt_usize(inv, "index").unwrap_or(usize::MAX);
                    let edits = perox_slint::edit::add_children_slot(&text, &comp, &parent, index).map_err(|e| err(e.to_string()))?;
                    apply_form_edits(s, id, edits);
                    return Ok(json!({ "added": "@children", "in": path_key(&parent) }));
                }
                // Like the designer: non-visual elements live at the top of the root.
                let (parent, default_index) = if perox_designer::catalog::is_non_visual(kind) {
                    let lead = doc.component(&comp).map_or(0, |c| {
                        c.root.children.iter().take_while(|e| perox_designer::catalog::is_non_visual(&e.kind)).count()
                    });
                    (Vec::new(), if kind == "MenuBar" { 0 } else { lead })
                } else {
                    (widget_path(&doc, &comp, inv.str_arg("parent")?)?, usize::MAX)
                };
                let index = opt_usize(inv, "index").unwrap_or(default_index);
                let mut props = Vec::new();
                for (k, v) in inv.args["props"].as_object().into_iter().flatten() {
                    let src = v.as_str().ok_or_else(|| err(format!("`{k}` must be a string with a .slint expression")))?;
                    props.push((k.clone(), src.to_owned()));
                }
                // Templates (ToolBar): a real element with its defaults and name.
                let template = perox_designer::catalog::template(kind);
                if template.is_some() {
                    for (k, v) in perox_designer::catalog::defaults(kind).into_iter().rev() {
                        if !props.iter().any(|(p, _)| *p == k) {
                            props.insert(0, (k, v));
                        }
                    }
                }
                let canvas = kind == "Canvas";
                let kind = template.map_or(kind, |(real, _)| real);
                // Like the designer: widgets code talks to get an id.
                let auto = match template {
                    Some((_, name)) => doc.component(&comp).map(|c| {
                        if c.find(name).is_none() { name.to_owned() } else { perox_designer::unique_id(c, name) }
                    }),
                    None => doc
                        .component(&comp)
                        .filter(|_| perox_designer::catalog::wants_id(kind))
                        .map(|c| perox_designer::unique_id(c, &perox_designer::catalog::snake(kind))),
                };
                let wid = inv.args["id"].as_str().or(auto.as_deref());
                let mut work = text.clone();
                perox_designer::free::step(&mut work, perox_slint::edit::add_element(&text, &comp, &parent, index, kind, wid, &props))
                    .map_err(|e| err(e.to_string()))?;
                perox_designer::free::import(&mut work, kind);
                // A project or library component: import it, as the designer does.
                let file = resolve(s, inv, "path")?;
                let ui_dir = perox_designer::translations::location(file.parent().unwrap_or(std::path::Path::new("."))).0;
                if let Some(from) = perox_designer::components::offers(&file, &text, &comp, &ui_dir)
                    .into_iter()
                    .find(|o| o.name == kind)
                    .and_then(|o| o.from)
                {
                    let e = perox_slint::edit::ensure_import(&work, kind, &from);
                    work = perox_slint::edit::apply(&work, &e);
                }
                if props.iter().any(|(_, v)| v.contains("Palette.")) {
                    let e = perox_slint::edit::ensure_import(&work, "Palette", perox_designer::catalog::STD_WIDGETS);
                    work = perox_slint::edit::apply(&work, &e);
                }
                if canvas && let Some(w) = wid {
                    work = perox_designer::canvas::wire(&work, &comp, w).map_err(|e| err(e.to_string()))?;
                }
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &work));
                let count = doc.component(&comp).and_then(|c| c.at(&parent)).map_or(0, |p| p.children.len());
                let mut path = parent;
                path.push(index.min(count));
                Ok(json!({ "added": path_key(&path), "id": wid }))
            },
        ),
        (
            CommandSpec::new("designer.app_style", "App Style", Access::Edit)
                .description("Gets or sets the style and colour scheme the app is built with, kept in ui/slint-project.json (metadata.peroxide) and applied by perox_slint::build::compile. style: fluent, cupertino, material, cosmic, or null for each OS's own. color_scheme: light, dark, or null to follow the system. Omit both to just read them. Rebuild for the running app to change.")
                .params(path_schema(json!({
                    "style": { "type": ["string", "null"] },
                    "color_scheme": { "type": ["string", "null"] }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let mut ui_settings = perox_slint::project::settings_for(&dir);
                let obj = inv.args.as_object();
                let has = |k: &str| obj.is_some_and(|o| o.contains_key(k));
                if has("style") || has("color_scheme") {
                    if has("style") {
                        ui_settings.style = match inv.args["style"].as_str() {
                            Some(st) if perox_slint::project::STYLES.contains(&st) => Some(st.to_owned()),
                            Some(st) => return Err(err(format!("Unknown style `{st}`: one of {}", perox_slint::project::STYLES.join(", ")))),
                            None => None,
                        };
                    }
                    if has("color_scheme") {
                        ui_settings.color_scheme = match inv.args["color_scheme"].as_str() {
                            Some(c) if perox_slint::project::COLOR_SCHEMES.contains(&c) => Some(c.to_owned()),
                            Some(c) => return Err(err(format!("Unknown colour scheme `{c}`: light or dark"))),
                            None => None,
                        };
                    }
                    let file = perox_slint::project::find(&dir).unwrap_or_else(|| dir.join(perox_slint::project::FILE_NAME));
                    let old = std::fs::read_to_string(&file).unwrap_or_default();
                    std::fs::write(&file, perox_slint::project::write(&old, &ui_settings)).map_err(|e| err(e.to_string()))?;
                    // Open designers pick it up on their next read; make it now.
                    for d in s.docs.list.iter_mut() {
                        if let Some(des) = d.designer.as_mut() {
                            let _ = des.set_app_ui(&ui_settings);
                        }
                    }
                }
                Ok(json!({ "style": ui_settings.style, "color_scheme": ui_settings.color_scheme }))
            },
        ),
        (
            CommandSpec::new("i18n.strings", "Translatable Strings", Access::Read)
                .description("The translatable (@tr) strings of the .slint files in a form's ui/ folder, each with its context (the component) and its translation in every language (null: not translated yet). Files: ui/translations/<lang>/LC_MESSAGES/<crate>.po, bundled into the app by the build.")
                .params(path_schema(json!({}))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let (ui, domain) = perox_designer::translations::location(path.parent().unwrap_or(std::path::Path::new(".")));
                let langs = perox_slint::i18n::languages(&ui);
                let pos: Vec<_> = langs.iter().map(|l| {
                    std::fs::read_to_string(perox_slint::i18n::po_path(&ui, l, &domain)).map(|t| perox_slint::i18n::parse_po(&t)).unwrap_or_default()
                }).collect();
                let strings: Vec<_> = perox_slint::i18n::extract_dir(&ui).into_iter().map(|m| {
                    let tr: serde_json::Map<String, serde_json::Value> = langs.iter().zip(&pos)
                        .map(|(l, po)| (l.clone(), po.get(&m.context, &m.id).map_or(serde_json::Value::Null, |t| t.into())))
                        .collect();
                    json!({ "context": m.context, "text": m.id, "plural": m.plural, "translations": tr })
                }).collect();
                Ok(json!({ "domain": domain, "languages": langs, "strings": strings }))
            },
        ),
        (
            CommandSpec::new("i18n.add_language", "Add Language", Access::Edit)
                .description("Adds a language (de, fr, pt_BR, …): writes its .po with every translatable string, untranslated. Also brings existing languages up to date with the forms (new strings added, gone ones dropped). Save the forms first.")
                .params(path_schema(json!({ "language": { "type": "string" } }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let lang = inv.str_arg("language")?;
                if !perox_slint::i18n::is_language_tag(lang) {
                    return Err(err("`language` is a tag like de, fr or pt_BR."));
                }
                let (ui, domain) = perox_designer::translations::location(path.parent().unwrap_or(std::path::Path::new(".")));
                let written = perox_slint::i18n::sync(&ui, &domain, Some(lang)).map_err(|e| err(e.to_string()))?;
                Ok(json!({ "written": written }))
            },
        ),
        (
            CommandSpec::new("i18n.translate", "Translate String", Access::Edit)
                .description("Sets one string's translation in a language and saves its .po. `context` is the component the text is in (see i18n.strings); default: the only string with that text.")
                .params(path_schema(json!({
                    "language": { "type": "string" },
                    "text": { "type": "string", "description": "The text as written in the form" },
                    "context": { "type": "string" },
                    "translation": { "type": "string" }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let lang = inv.str_arg("language")?;
                let text = inv.str_arg("text")?;
                let (ui, domain) = perox_designer::translations::location(path.parent().unwrap_or(std::path::Path::new(".")));
                if !perox_slint::i18n::languages(&ui).iter().any(|l| l == lang) {
                    return Err(err(format!("No `{lang}` yet: i18n.add_language first.")));
                }
                let msgs = perox_slint::i18n::extract_dir(&ui);
                let matching: Vec<_> = msgs.iter().filter(|m| m.id == text && inv.args["context"].as_str().is_none_or(|c| c == m.context)).collect();
                let m = match matching.as_slice() {
                    [one] => (*one).clone(),
                    [] => return Err(err(format!("No translatable string `{text}` (save the form, and is it @tr?)."))),
                    _ => return Err(err("That text is in several components: give `context`.")),
                };
                let po_path = perox_slint::i18n::po_path(&ui, lang, &domain);
                let mut po = std::fs::read_to_string(&po_path).map(|t| perox_slint::i18n::parse_po(&t)).unwrap_or_default();
                po.set(&m.context, &m.id, inv.str_arg("translation")?);
                std::fs::write(&po_path, perox_slint::i18n::write_po(lang, &msgs, &po)).map_err(|e| err(e.to_string()))?;
                Ok(json!({ "context": m.context, "saved": po_path }))
            },
        ),
        (
            CommandSpec::new("designer.make_translatable", "Make All Text Translatable", Access::Edit)
                .description("Wraps every plain text, title and placeholder-text string of a component in @tr(…), so it can be translated. Returns how many it changed.")
                .params(path_schema(json!({ "component": { "type": "string" } }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let (new, n) = perox_designer::translations::make_translatable(&text, &comp).map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                Ok(json!({ "changed": n }))
            },
        ),
        (
            CommandSpec::new("data.database", "Design Database", Access::Edit)
                .description("Gets or sets the SQLite file (relative to the crate root, created if missing) that the designer runs bound queries on, to show data while designing. Kept in ui/slint-project.json. The app opens its own database at run time with perox_ui::data::Database::open.")
                .params(path_schema(json!({ "file": { "type": "string", "description": "e.g. data/app.db; omit to read" } }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let mut ui_settings = perox_slint::project::settings_for(&dir);
                if let Some(f) = inv.args["file"].as_str() {
                    let root = perox_designer::data::crate_root(&dir).ok_or_else(|| err("No Cargo.toml above the form."))?;
                    let full = normalize(&root.join(f));
                    if !full.starts_with(&root) {
                        return Err(err("The database must be inside the project."));
                    }
                    if let Some(d) = full.parent() {
                        std::fs::create_dir_all(d).map_err(|e| err(e.to_string()))?;
                    }
                    perox_ui::data::Database::open(&format!("sqlite:{}", full.display())).map_err(|e| err(e.to_string()))?;
                    ui_settings.database = Some(f.to_owned());
                    let file = perox_slint::project::find(&dir).unwrap_or_else(|| dir.join(perox_slint::project::FILE_NAME));
                    let old = std::fs::read_to_string(&file).unwrap_or_default();
                    std::fs::write(&file, perox_slint::project::write(&old, &ui_settings)).map_err(|e| err(e.to_string()))?;
                    for d in s.docs.list.iter_mut() {
                        if let Some(des) = d.designer.as_mut() {
                            let _ = des.set_app_ui(&ui_settings);
                        }
                    }
                }
                Ok(json!({ "database": ui_settings.database }))
            },
        ),
        (
            CommandSpec::new("data.query", "Query Design Database", Access::Read)
                .description("Runs a SELECT on the design database (data.database) and returns its columns and up to 200 rows as text.")
                .params(path_schema(json!({ "sql": { "type": "string" } }))),
            |s, inv| {
                let db = design_db(s, inv)?;
                let t = db.query(inv.str_arg("sql")?).map_err(|e| err(e.to_string()))?;
                let total = t.len();
                Ok(json!({ "columns": t.columns, "rows": t.rows.into_iter().take(200).collect::<Vec<_>>(), "total": total }))
            },
        ),
        (
            CommandSpec::new("data.execute", "Change Design Database", Access::Execute)
                .description("Runs SQL that changes the design database (CREATE TABLE, INSERT, …; several statements are fine) and returns the rows changed.")
                .params(path_schema(json!({ "sql": { "type": "string" } }))),
            |s, inv| {
                let db = design_db(s, inv)?;
                let n = db.execute(inv.str_arg("sql")?).map_err(|e| err(e.to_string()))?;
                Ok(json!({ "changed": n }))
            },
        ),
        (
            CommandSpec::new("designer.bind_query", "Bind to Query", Access::Edit)
                .description("Binds a StandardTableView, StandardListView or ComboBox to a SQL query: exposes its data (<id>_rows / <id>_model), declares `<id>_query`, writes a table's columns from the query's, and turns on perox-ui's sqlite feature. The build then generates `<Window>::load_<id>(&db)`, which runs the query and fills the widget. The designer shows the rows from the design database.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "sql": { "type": "string" }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let wpath = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let sql = inv.str_arg("sql")?;
                let columns = design_db(s, inv).ok().and_then(|db| db.query(sql).ok()).map(|t| t.columns);
                let new = perox_designer::data::bind(&text, &comp, &wpath, sql, columns.as_deref()).map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                let path = resolve(s, inv, "path")?;
                let mut cargo = None;
                if let Some(root) = perox_designer::data::crate_root(path.parent().unwrap_or(std::path::Path::new("."))) {
                    let f = root.join("Cargo.toml");
                    if let Ok(t) = std::fs::read_to_string(&f)
                        && let Some(n) = perox_designer::data::with_sqlite_feature(&t)
                    {
                        std::fs::write(&f, n).map_err(|e| err(e.to_string()))?;
                        cargo = Some("perox-ui feature sqlite turned on");
                    }
                }
                let new_doc = perox_slint::parse(&new);
                let b = new_doc.component(&comp).map(|c| perox_slint::data::bindings(c, Some(&new))).unwrap_or_default();
                let bound = b.iter().find(|b| new_doc.component(&comp).and_then(|c| c.find(&b.id)) == Some(wpath.clone()));
                Ok(json!({
                    "query_property": bound.map(|b| &b.query_property),
                    "data_property": bound.map(|b| &b.data_property),
                    "load_method": bound.map(|b| format!("load_{}", b.id.replace('-', "_"))),
                    "columns": columns,
                    "cargo": cargo
                }))
            },
        ),
        (
            CommandSpec::new("designer.tab_order", "Tab Order", Access::Edit)
                .description("The focusable widgets in tab order (Slint uses the order of the .slint file), or with `widget` and `move` (\"earlier\" / \"later\"), moves one a place. Moving works within a freely placed container (nothing moves on screen); in a layout the tab order is the layout's order, so it says so instead.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "move": { "type": "string", "enum": ["earlier", "later"] },
                    "show": { "type": "boolean", "description": "Number the widgets on the designer's canvas (or stop)" }
                }))),
            |s, inv| {
                let (id, mut text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                if let (Some(w), Some(m)) = (inv.args["widget"].as_str(), inv.args["move"].as_str()) {
                    let path = widget_path(&doc, &comp, w)?;
                    let (new, _) = perox_designer::taborder::step(&text, &comp, &path, m == "earlier").map_err(err)?;
                    apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                    text = new;
                }
                if let Some(show) = inv.args["show"].as_bool()
                    && let Some(d) = s.docs.get_mut(id).and_then(|d| d.designer.as_mut())
                {
                    d.show_tab_order = show;
                }
                let doc = perox_slint::parse(&text);
                let c = doc.component(&comp).ok_or_else(|| err("No such component."))?;
                let order: Vec<_> = perox_designer::taborder::order(c)
                    .iter()
                    .map(|p| {
                        let el = c.at(p);
                        json!({ "path": path_key(p), "id": el.and_then(|e| e.id.clone()), "kind": el.map(|e| e.kind.clone()) })
                    })
                    .collect();
                Ok(json!({ "order": order }))
            },
        ),
        (
            CommandSpec::new("designer.components", "Project Components", Access::Read)
                .description("Components a form can use besides Slint's own: reusable components in this file, exported ones from the project's other .slint files, and its libraries' (library-paths in ui/slint-project.json). Each with what designer.add_widget imports it from.")
                .params(path_schema(json!({ "component": { "type": "string" } }))),
            |s, inv| {
                let (_, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let file = resolve(s, inv, "path")?;
                let ui_dir = perox_designer::translations::location(file.parent().unwrap_or(std::path::Path::new("."))).0;
                let list: Vec<_> = perox_designer::components::offers(&file, &text, &comp, &ui_dir)
                    .into_iter()
                    .map(|o| json!({ "name": o.name, "from": o.from, "group": o.group }))
                    .collect();
                Ok(json!({ "components": list }))
            },
        ),
        (
            CommandSpec::new("designer.add_library", "Add Component Library", Access::Edit)
                .description("Adds a .slint component library to the project (library-paths in ui/slint-project.json): a .slint file (import { X } from \"@name\") or a folder (import { X } from \"@name/file.slint\"). Its components appear in the palette and designer.components; the build and the preview find them.")
                .params(path_schema(json!({
                    "name": { "type": "string", "description": "Letters, digits, - and _" },
                    "library": { "type": "string", "description": "The library's .slint file or folder (absolute, or relative to the project)" }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let name = inv.str_arg("name")?;
                if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                    return Err(err("`name` is letters, digits, - and _."));
                }
                let root = s.project.as_ref().map(|p| p.root.clone()).ok_or_else(|| err("No project is open."))?;
                let lib = normalize(&root.join(inv.str_arg("library")?));
                if !lib.exists() {
                    return Err(err(format!("{} doesn't exist.", lib.display())));
                }
                let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let file = perox_slint::project::find(&dir).unwrap_or_else(|| dir.join(perox_slint::project::FILE_NAME));
                let json_dir = file.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let rel = pathdiff_rel(&lib, &json_dir);
                let old = std::fs::read_to_string(&file).unwrap_or_default();
                std::fs::write(&file, perox_slint::project::add_library(&old, name, &rel)).map_err(|e| err(e.to_string()))?;
                Ok(json!({ "library": name, "path": rel, "import_from": format!("@{name}") }))
            },
        ),
        (
            CommandSpec::new("designer.declare_property", "Declare Property", Access::Edit)
                .description("Declares a property on a component's root: `in` for what forms using a component set on it, `out` for what it reports, `in-out` for both (and what Rust can read and write on a window), `private` for its own use. Type is a Slint type (string, length, int, float, bool, color, brush, image, [string], a struct…); `default` is a .slint expression. Remove one with designer.remove_declaration.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "name": { "type": "string" },
                    "type": { "type": "string" },
                    "visibility": { "type": "string", "enum": ["in", "out", "in-out", "private"], "description": "Default: in" },
                    "default": { "type": "string" }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let name = inv.str_arg("name")?;
                if doc.component(&comp).and_then(|c| c.property(name)).is_some() {
                    return Err(err(format!("`{name}` is declared already.")));
                }
                let vis = inv.args["visibility"].as_str().unwrap_or("in");
                let edits = perox_slint::edit::declare_property(&text, &comp, vis, name, inv.str_arg("type")?, inv.args["default"].as_str())
                    .map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "declared": name }))
            },
        ),
        (
            CommandSpec::new("designer.declare_callback", "Declare Callback", Access::Edit)
                .description("Declares a callback on a component's root: an event the component raises (`callback picked(index: int);`), which forms using it handle like a button's clicked, and which a window's Rust handlers trait gets as a method. `params` is a list of [name, type].")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "name": { "type": "string" },
                    "params": { "type": "array", "items": { "type": "array", "items": { "type": "string" } } }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let params: Vec<(String, String)> = inv.args["params"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| Some((p.get(0)?.as_str()?.to_owned(), p.get(1)?.as_str()?.to_owned())))
                    .collect();
                let decl: Vec<(Option<&str>, &str)> = params.iter().map(|(n, t)| (Some(n.as_str()), t.as_str())).collect();
                let edits = perox_slint::edit::add_callback(&text, &comp, inv.str_arg("name")?, &decl).map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "ok": true }))
            },
        ),
        (
            CommandSpec::new("designer.remove_declaration", "Remove Declaration", Access::Edit)
                .description("Removes a property or callback declaration from a component's root (what code or other forms no longer use). Bindings that still use it will show as errors in designer.tree.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "name": { "type": "string" }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let name = inv.str_arg("name")?;
                let c = doc.component(&comp).ok_or_else(|| err("No such component."))?;
                let edits = if c.property(name).is_some() {
                    perox_slint::edit::remove_property_decl(&text, &comp, name)
                } else if c.callback(name).is_some() {
                    perox_slint::edit::remove_callback(&text, &comp, name)
                } else {
                    return Err(err(format!("`{name}` isn't declared on {comp}.")));
                }
                .map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "removed": name }))
            },
        ),
        (
            CommandSpec::new("designer.set_header", "Repeat or Show If", Access::Edit)
                .description("Makes an element repeat for each item of a model (`for item in root.items`, also `for item[i] in …`) or show only when a condition holds (`if root.show`), or, with null, a plain element again. Inside, `item` (and `i`) can be used in its properties.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "header": { "type": ["string", "null"] }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let edits = perox_slint::edit::set_header(&text, &comp, &path, inv.args["header"].as_str())
                    .map_err(|_| err("`header` is `for item in model` or `if condition` (or null)."))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "ok": true }))
            },
        ),
        (
            CommandSpec::new("designer.resources", "Form Resources", Access::Read)
                .description("Images in a .slint file's folder (paths for @image-url, relative to the file) and the fonts it imports, with their family names.")
                .params(path_schema(json!({}))),
            |s, inv| {
                let (_, text) = form_text(s, inv)?;
                let path = resolve(s, inv, "path")?;
                let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let doc = perox_slint::parse(&text);
                let fonts: Vec<_> = perox_designer::resources::imported_fonts(&doc, &dir)
                    .into_iter()
                    .map(|(family, file)| json!({ "family": family, "file": file }))
                    .collect();
                Ok(json!({ "images": perox_designer::resources::images(&dir), "fonts": fonts }))
            },
        ),
        (
            CommandSpec::new("designer.add_resource", "Add Image or Font", Access::Edit)
                .description("Brings an image (png, jpg, svg, gif, webp) or font (ttf, otf) file into the .slint file's folder (copied to images/ or fonts/ unless it's already inside) so the app embeds it. Images: returns the `@image-url(...)` expression to set as `source` or `icon`. Fonts: adds `import \"fonts/X.ttf\";` to the file and returns the family name for `font-family` / `default-font-family`.")
                .params(path_schema(json!({ "file": { "type": "string", "description": "Absolute path of the image or font file" } }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let path = resolve(s, inv, "path")?;
                let dir = path.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
                let file = std::path::PathBuf::from(inv.str_arg("file")?);
                use perox_designer::resources as r;
                // Only real images and fonts: this reads outside the project.
                if r::is_font(&file) {
                    let family = r::font_family(&file).ok_or_else(|| err("Not a font file Slint can read."))?;
                    let rel = r::import_file(&file, &dir, "fonts").map_err(|e| err(e.to_string()))?;
                    apply_form_edits(s, id, perox_slint::edit::ensure_file_import(&text, &rel));
                    Ok(json!({ "file": rel, "family": family }))
                } else if r::is_image(&file) {
                    let head = std::fs::read(&file).map_err(|e| err(e.to_string()))?;
                    let svg = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
                    let ok = if svg { String::from_utf8_lossy(&head[..head.len().min(512)]).contains("<svg") || String::from_utf8_lossy(&head[..head.len().min(512)]).contains("<?xml") } else { image::guess_format(&head).is_ok() };
                    if !ok {
                        return Err(err("Not an image file."));
                    }
                    let rel = r::import_file(&file, &dir, "images").map_err(|e| err(e.to_string()))?;
                    Ok(json!({ "file": rel, "expression": r::image_url(&rel) }))
                } else {
                    Err(err("Give an image (png, jpg, svg, gif, webp, bmp) or a font (ttf, otf, ttc)."))
                }
            },
        ),
        (
            CommandSpec::new("designer.actions", "Form Actions", Access::Read)
                .description("A component's actions: callbacks without parameters, which menu items and buttons share. For each: name, its `<name>_enabled` property if it can be disabled, and the elements that run it (paths).")
                .params(path_schema(json!({ "component": { "type": "string" } }))),
            |s, inv| {
                let (_, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let c = doc.component(&comp).ok_or_else(|| err(format!("No component `{comp}`.")))?;
                let list: Vec<_> = perox_designer::actions::list(c)
                    .into_iter()
                    .map(|a| json!({ "name": a.name, "enabled": a.enabled, "users": a.users.iter().map(|p| path_key(p)).collect::<Vec<_>>() }))
                    .collect();
                Ok(json!({ "actions": list }))
            },
        ),
        (
            CommandSpec::new("designer.set_action", "Set Form Action", Access::Edit)
                .description("Declares an action (`callback name();`, a method of the generated <Component>Handlers trait) if it's missing, and sets whether it can be disabled: `in-out property <bool> name_enabled: true;` that every element running it binds `enabled` to, so Rust's `set_name_enabled(false)` greys out its menu items and buttons together.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "name": { "type": "string" },
                    "can_disable": { "type": "boolean", "description": "Default: false" }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let name = inv.str_arg("name")?.replace('-', "_");
                let on = inv.args["can_disable"].as_bool().unwrap_or(false);
                let new = perox_designer::actions::add(&text, &comp, &name, false)
                    .and_then(|t| perox_designer::actions::set_can_disable(&t, &comp, &name, on))
                    .map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                Ok(json!({ "action": name, "enabled": on.then(|| perox_designer::actions::enabled_name(&name)) }))
            },
        ),
        (
            CommandSpec::new("designer.use_action", "Use Form Action", Access::Edit)
                .description("Makes an element run an action: its trigger event (clicked, activated, …) forwards to the action's callback and its `enabled` follows the action's flag. Give `widget` for an existing element, or `parent` and `kind` (MenuItem in a Menu, Button in a toolbar) to add a new one captioned after the action.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "action": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "parent": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "kind": { "type": "string", "description": "With parent: MenuItem or Button" }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let action = inv.str_arg("action")?;
                let (new, path) = if let Some(w) = inv.args["widget"].as_str() {
                    let path = widget_path(&doc, &comp, w)?;
                    (perox_designer::actions::use_action(&text, &comp, &path, action).map_err(|e| err(e.to_string()))?, path)
                } else {
                    let parent = widget_path(&doc, &comp, inv.str_arg("parent")?)?;
                    let kind = inv.args["kind"].as_str().unwrap_or("Button");
                    perox_designer::actions::add_user(&text, &comp, &parent, kind, action).map_err(|e| err(e.to_string()))?
                };
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                Ok(json!({ "widget": path_key(&path) }))
            },
        ),
        (
            CommandSpec::new("designer.set_property", "Set Element Property", Access::Edit)
                .description("Sets an element property to a .slint expression (\"\\\"text\\\"\", \"12px\", \"true\", \"center\", \"root.width / 2\"), or removes it when value is null. Only that value's text changes.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "name": { "type": "string" },
                    "value": { "type": ["string", "null"] }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let name = inv.str_arg("name")?;
                let edits = match inv.args["value"].as_str() {
                    Some(v) => perox_slint::edit::set_property(&text, &comp, &path, name, v),
                    None => perox_slint::edit::remove_property(&text, &comp, &path, name),
                }
                .map_err(|e| err(e.to_string()))?;
                // Theme colours need `Palette` imported, as the designer does.
                let mut work = perox_slint::edit::apply(&text, &edits);
                if inv.args["value"].as_str().is_some_and(|v| v.contains("Palette.")) {
                    let e = perox_slint::edit::ensure_import(&work, "Palette", perox_designer::catalog::STD_WIDGETS);
                    work = perox_slint::edit::apply(&work, &e);
                }
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &work));
                Ok(json!({ "ok": true }))
            },
        ),
        (
            CommandSpec::new("designer.remove_widget", "Remove Element", Access::Edit)
                .description("Removes an element (and its children) from a component.")
                .params(path_schema(json!({ "component": { "type": "string" }, "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }) }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let edits = perox_slint::edit::remove_element(&text, &comp, &path).map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "removed": path_key(&path) }))
            },
        ),
        (
            CommandSpec::new("designer.move_widget", "Move Element", Access::Edit)
                .description("Moves an element to child position `index` of `parent` (positions as before the move).")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "parent": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "index": { "type": "integer", "minimum": 0 }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let from = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let parent = widget_path(&doc, &comp, inv.str_arg("parent")?)?;
                let index = opt_usize(inv, "index").unwrap_or(usize::MAX);
                let edits = perox_slint::edit::move_element(&text, &comp, &from, &parent, index).map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "ok": true }))
            },
        ),
        (
            CommandSpec::new("designer.set_handler", "Set Event Handler", Access::Edit)
                .description("Wires an element's callback (clicked, edited, accepted, toggled, changed, selected, …) to a root callback `method` (declared if missing, and so a method of the generated <Component>Handlers trait), or unwires it when method is null. Or, with `code` instead of `method`, gives it .slint statements to run (`root.current = i; root.picked(i);`), for behaviour that stays in the form; `params` names the event's arguments.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "event": { "type": "string" },
                    "method": { "type": ["string", "null"] },
                    "code": { "type": "string" },
                    "params": { "type": "array", "items": { "type": "string" } }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let event = inv.str_arg("event")?;
                if let Some(code) = inv.args["code"].as_str() {
                    let params: Vec<&str> = inv.args["params"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
                    let edits = perox_slint::edit::set_handler(&text, &comp, &path, event, &params, code).map_err(|e| err(e.to_string()))?;
                    apply_form_edits(s, id, edits);
                    return Ok(json!({ "ok": true }));
                }
                let edits = match inv.args["method"].as_str() {
                    Some(m) => perox_designer::wire_event(&text, &comp, &path, event, m).map(|new| perox_slint::edit::diff(&text, &new)),
                    None => perox_slint::edit::remove_handler(&text, &comp, &path, event),
                }
                .map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, edits);
                Ok(json!({ "ok": true }))
            },
        ),
        (
            CommandSpec::new("designer.expose", "Use Property from Code", Access::Edit)
                .description("Makes an element's property readable and writable from Rust: names the element if needed and declares `in-out property <type> <id>_<prop> <=> <id>.<prop>;`, so the generated component gets get_<id>_<prop>() and set_<id>_<prop>(). With expose: false, removes that declaration.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "name": { "type": "string", "description": "The property, e.g. text, checked, value" },
                    "as": { "type": "string", "description": "The root property's name (default <id>_<prop>); e.g. a component passing its LineEdit's text through as `text`" },
                    "expose": { "type": "boolean", "description": "Default true" }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let prop = inv.str_arg("name")?;
                if inv.args["expose"].as_bool() == Some(false) {
                    let new = perox_designer::unexpose_property(&text, &comp, &path, prop).map_err(|e| err(e.to_string()))?;
                    apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                    return Ok(json!({ "exposed": false }));
                }
                let as_name = inv.args["as"].as_str();
                // A project component's property: its type from the declaration.
                let file = resolve(s, inv, "path")?;
                let custom_ty = doc
                    .component(&comp)
                    .and_then(|c| c.at(&path))
                    .and_then(|el| perox_designer::components::definition(&file, &text, &el.kind))
                    .and_then(|d| d.properties.into_iter().find(|p| p.name == prop && p.visibility != "private"))
                    .and_then(|p| p.ty);
                let new = perox_designer::expose_property_as(&text, &comp, &path, prop, as_name, custom_ty.as_deref()).map_err(|e| err(e.to_string()))?;
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &new));
                let doc = perox_slint::parse(&new);
                let name = as_name.map(str::to_owned).or_else(|| doc
                    .component(&comp)
                    .and_then(|c| {
                        let el = c.at(&path)?;
                        c.exposing(&format!("{}.{prop}", el.id.as_ref()?)).map(|p| p.name.clone())
                    }))
                    .unwrap_or_default();
                let r = perox_slint::codegen::rust_name(&name);
                Ok(json!({ "exposed": true, "property": name, "rust": [format!("get_{r}"), format!("set_{r}")] }))
            },
        ),
        (
            CommandSpec::new("designer.go_to_handler", "Go to Element Handler", Access::Edit)
                .description("Like double-clicking an event in the designer: wires the element's callback to `<id>_<event>` if it isn't yet, then shows the Rust method (adding it to the `impl <Component>Handlers` block).")
                .params(path_schema(json!({ "component": { "type": "string" }, "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }), "event": { "type": "string" } }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let event = inv.str_arg("event")?;
                let mut d = perox_designer::Designer::new();
                d.component = Some(comp);
                let actions = d.go_to_handler(&text, &path, event);
                if let Some(e) = d.error {
                    return Err(err(e));
                }
                let mut opened = Value::Null;
                for a in actions {
                    match a {
                        perox_designer::Action::Edit(edits) => apply_form_edits(s, id, edits),
                        perox_designer::Action::OpenHandler { form, method, params } => {
                            opened = open_handler(s, &form, &method, &params)?;
                        }
                        _ => {}
                    }
                }
                Ok(opened)
            },
        ),
        (
            CommandSpec::new("designer.break_layout", "Break Layout", Access::Edit)
                .description("Qt-style Break Layout: a layout (VerticalBox, HorizontalBox, GridBox…) becomes a Rectangle, or a window/tab/rectangle holding a single layout takes over its children. Every element keeps its place, now as x, y, width, height. The form must be showing in the designer.")
                .params(path_schema(json!({ "component": { "type": "string" }, "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }) }))),
            |s, inv| relayout_command(s, inv, None),
        ),
        (
            CommandSpec::new("designer.lay_out", "Lay Out", Access::Edit)
                .description("Puts a container's freely placed children into a layout, ordered by where they sit: direction vertical, horizontal or grid. A nested Rectangle becomes the layout; a window or tab gets a new layout inside. The form must be showing in the designer.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "direction": { "type": "string", "enum": ["vertical", "horizontal", "grid"] }
                }))),
            |s, inv| {
                let dir = match inv.str_arg("direction")? {
                    "vertical" => perox_designer::free::Direction::Vertical,
                    "horizontal" => perox_designer::free::Direction::Horizontal,
                    "grid" => perox_designer::free::Direction::Grid,
                    other => return Err(err(format!("Unknown direction `{other}`."))),
                };
                relayout_command(s, inv, Some(dir))
            },
        ),
        (
            CommandSpec::new("designer.select", "Select Element", Access::Read)
                .description("Shows a form in the designer with an element selected (for screenshots and pointing things out).")
                .params(path_schema(json!({ "component": { "type": "string" }, "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }) }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                if let Some(doc) = s.docs.get_mut(id) {
                    doc.design_view = true;
                    if let Some(d) = doc.designer.as_mut() {
                        d.component = Some(comp);
                        d.selected = Some(path.clone());
                    }
                }
                Ok(json!({ "selected": path_key(&path) }))
            },
        ),
        // ── projects ──
        (
            CommandSpec::new("project.open", "Open Project", Access::Edit)
                .description("Opens the Cargo project or workspace containing a folder or file. Starts rust-analyzer for it.")
                .params(json!({ "type": "object", "required": ["path"], "properties": { "path": { "type": "string", "description": "Absolute path" } } })),
            |s, inv| open_project(s, Path::new(inv.str_arg("path")?)),
        ),
        (
            CommandSpec::new("project.new", "New GUI App", Access::Edit)
                .description("Creates a GUI app (a window form, its code-behind and build.rs) in a new folder, opens it, and shows the form in the designer. `path` is the new folder (absolute); its name becomes the package name.")
                .params(json!({ "type": "object", "required": ["path"], "properties": { "path": { "type": "string", "description": "Absolute path of a folder that doesn't exist yet (or is empty)" } } })),
            |s, inv| {
                let dir = PathBuf::from(inv.str_arg("path")?);
                if !dir.is_absolute() {
                    return Err(err("Give an absolute path for the new app's folder."));
                }
                let sdk = perox_build::tools::sdk_dir()
                    .ok_or_else(|| err("Can't find the PeroxIDE runtime crates (perox-ui). Reinstall, or set PEROXIDE_SDK."))?;
                let form = crate::templates::create_gui_app(&dir, &sdk).map_err(err)?;
                open_project(s, &dir)?;
                open_file(s, &form, None, None)?;
                Ok(json!({ "created": dir, "form": form }))
            },
        ),
        (
            CommandSpec::new("form.new", "New Window or Dialog", Access::Edit)
                .description("Adds a window or dialog to the project: writes a new .slint file (the type name comes from the file name: settings_dialog.slint → SettingsDialog), exports it from the main .slint file so Rust can use it, and opens it in the designer. Dialogs get OK and Cancel and an `exec` method in Rust.")
                .params(path_schema(json!({
                    "kind": { "type": "string", "enum": ["window", "dialog", "component"], "description": "Default: dialog. A component is a reusable part for other forms (offered in the palette), not a window." },
                    "inherits": { "type": "string", "description": "Instead of a blank window or dialog: inherit this exported component (form inheritance). It starts with everything that form has; its own elements are added after them, and the base's properties can be set on it." }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                if path.extension().is_none_or(|e| e != "slint") {
                    return Err(err("The file must end in .slint."));
                }
                let kind = match inv.args["kind"].as_str() {
                    Some("window") => crate::templates::FormKind::Window,
                    Some("component") => crate::templates::FormKind::Component,
                    _ => crate::templates::FormKind::Dialog,
                };
                let name = match inv.args["inherits"].as_str() {
                    Some(base) => {
                        let root = s.project.as_ref().map(|p| p.root.join("ui")).ok_or_else(|| err("No project is open."))?;
                        let file = crate::templates::exported_forms(&root)
                            .into_iter()
                            .find(|(n, _)| n == base)
                            .map(|(_, f)| f)
                            .ok_or_else(|| err(format!("No exported component `{base}` in ui/.")))?;
                        crate::templates::add_inherited_form(&path, base, &file).map_err(err)?
                    }
                    None => crate::templates::add_form(&path, kind).map_err(err)?,
                };
                if let Some(p) = &s.project {
                    s.tree = Some(p.file_tree(20_000));
                    s.files = p.files(50_000);
                }
                open_file(s, &path, None, None)?;
                Ok(json!({ "created": rel(s, &path), "component": name }))
            },
        ),
        (CommandSpec::new("form.new_window_dialog", "New Window…", Access::Edit).ui_only(), |s, _| {
            s.new_form_requested = Some(crate::templates::FormKind::Window);
            Ok(Value::Null)
        }),
        (CommandSpec::new("form.new_component_dialog", "New Component…", Access::Edit).ui_only(), |s, _| {
            s.new_form_requested = Some(crate::templates::FormKind::Component);
            Ok(Value::Null)
        }),
        (CommandSpec::new("form.new_inherited_dialog", "New Inherited Form…", Access::Edit).ui_only(), |s, _| {
            let root = s.project.as_ref().map(|p| p.root.join("ui")).ok_or_else(|| err("No project is open."))?;
            s.inherit_picker = Some(crate::templates::exported_forms(&root));
            Ok(Value::Null)
        }),
        (CommandSpec::new("form.new_dialog_dialog", "New Dialog…", Access::Edit).ui_only(), |s, _| {
            s.new_form_requested = Some(crate::templates::FormKind::Dialog);
            Ok(Value::Null)
        }),
        (CommandSpec::new("project.new_dialog", "New GUI App…", Access::Edit).ui_only(), |s, _| {
            s.new_dialog_requested = true;
            Ok(Value::Null)
        }),
        (CommandSpec::new("project.open_dialog", "Open Project…", Access::Edit).ui_only(), |s, _| {
            s.open_dialog_requested = true;
            Ok(Value::Null)
        }),
        (
            CommandSpec::new("project.info", "Project Info", Access::Read)
                .description("The open project: root folder, name, packages and their targets (bin, lib, test…)."),
            |s, _| s.project.as_ref().map(project_info).ok_or_else(|| err("No project is open.")),
        ),
        (
            CommandSpec::new("project.files", "List Project Files", Access::Read)
                .description("Files in the project (relative paths), respecting .gitignore and skipping target/.")
                .params(json!({ "type": "object", "properties": { "limit": { "type": "integer", "default": 2000 } } })),
            |s, inv| {
                let p = s.project.as_ref().ok_or_else(|| err("No project is open."))?;
                Ok(json!(p.files(opt_usize(inv, "limit").unwrap_or(2000))))
            },
        ),
        // ── files ──
        (
            CommandSpec::new("file.open", "Open File", Access::Read)
                .description("Opens a file in an editor tab (or switches to it) and optionally moves the cursor to a line and column.")
                .params(path_schema(line_col.clone())),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                open_file(s, &path, opt_usize(inv, "line"), opt_usize(inv, "column"))?;
                Ok(json!({ "opened": rel(s, &path) }))
            },
        ),
        (
            CommandSpec::new("file.read", "Read File", Access::Read)
                .description("A file's text as the editor has it (including unsaved changes), optionally a range of lines, with line numbers.")
                .params(path_schema(json!({
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let text = match s.docs.by_path(&path).and_then(|id| s.docs.get(id)) {
                    Some(d) => d.editor.core.text(),
                    None => std::fs::read_to_string(&path).map_err(|e| err(format!("{}: {e}", path.display())))?,
                };
                let lines: Vec<&str> = text.lines().collect();
                let start = opt_usize(inv, "start_line").unwrap_or(1).max(1);
                let end = opt_usize(inv, "end_line").unwrap_or(lines.len()).min(lines.len());
                let body: String = (start..=end)
                    .filter_map(|n| lines.get(n - 1).map(|l| format!("{n:>5}  {l}\n")))
                    .collect();
                Ok(json!({ "path": rel(s, &path), "total_lines": lines.len(), "text": body }))
            },
        ),
        (
            CommandSpec::new("file.write", "Write File", Access::Edit)
                .description("Replaces a file's whole text. If it's open, the change goes into the editor (undoable, unsaved); otherwise the file is written (and created if needed).")
                .params(path_schema(json!({ "text": { "type": "string" } }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let text = inv.str_arg("text")?.to_owned();
                if let Some(id) = s.docs.by_path(&path) {
                    let doc = s.docs.get_mut(id).expect("by_path");
                    doc.editor.set_text(&text);
                    let edits = doc.editor.take_lsp_edits();
                    s.lsp.did_change(id, &s.docs, &edits);
                    return Ok(json!({ "updated_editor": rel(s, &path), "saved": false }));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| err(e.to_string()))?;
                }
                std::fs::write(&path, text).map_err(|e| err(e.to_string()))?;
                s.refresh_tree();
                Ok(json!({ "written": rel(s, &path) }))
            },
        ),
        (
            CommandSpec::new("editor.replace", "Replace Text", Access::Edit)
                .description("Replaces text between two positions (1-based line and column; end is exclusive) in a file, opening it if needed. Undoable; the file isn't saved.")
                .params(path_schema(json!({
                    "start_line": { "type": "integer" }, "start_column": { "type": "integer" },
                    "end_line": { "type": "integer" }, "end_column": { "type": "integer" },
                    "text": { "type": "string" }
                }))),
            |s, inv| {
                let path = resolve(s, inv, "path")?;
                let id = open_file(s, &path, None, None)?;
                let num = |n: &str| opt_usize(inv, n).ok_or_else(|| CommandError::InvalidArgs(format!("`{n}` is required")));
                let (sl, sc, el, ec) = (num("start_line")?, num("start_column")?, num("end_line")?, num("end_column")?);
                let text = inv.str_arg("text")?.to_owned();
                let doc = s.docs.get_mut(id).expect("just opened");
                let d = &doc.editor.core.doc;
                let start = d.char_of(Pos { line: sl.saturating_sub(1), col: sc.saturating_sub(1) });
                let end = d.char_of(Pos { line: el.saturating_sub(1), col: ec.saturating_sub(1) });
                doc.editor.apply_extra_edits(vec![Change::replace(start..end.max(start), text)]);
                let edits = doc.editor.take_lsp_edits();
                s.lsp.did_change(id, &s.docs, &edits);
                Ok(json!({ "edited": rel(s, &path) }))
            },
        ),
        (
            CommandSpec::new("editor.type", "Type Text", Access::Edit)
                .description("Types text into the active editor at the cursor, keystroke by keystroke, as a person would: brackets auto-close, Enter indents, and the completion popup opens. Use editor.completions to see it. Optionally move the cursor first (1-based line/column).")
                .params(json!({ "type": "object", "required": ["text"], "properties": {
                    "text": { "type": "string" }, "line": { "type": "integer" }, "column": { "type": "integer" }
                }})),
            |s, inv| {
                let id = s.active_doc.ok_or_else(|| err("No editor is active."))?;
                let text = inv.str_arg("text")?.to_owned();
                let doc = s.docs.get_mut(id).ok_or_else(|| err("No editor is active."))?;
                if let Some(line) = opt_usize(inv, "line") {
                    doc.editor.goto(line.saturating_sub(1), opt_usize(inv, "column").unwrap_or(1).saturating_sub(1));
                }
                let requests = doc.editor.type_text(&text, &s.recency);
                let edits = doc.editor.take_lsp_edits();
                s.lsp.did_change(id, &s.docs, &edits);
                for r in &requests {
                    s.lsp.request(id, &s.docs, r);
                }
                Ok(json!({ "typed": text.chars().count(), "requests": requests.len() }))
            },
        ),
        (
            CommandSpec::new("editor.completions", "Completion List", Access::Read)
                .description("What the active editor's completion popup offers right now, best first (empty if closed). Completions arrive a moment after typing.")
                .params(json!({ "type": "object", "properties": { "limit": { "type": "integer", "default": 20 } } })),
            |s, inv| {
                let doc = s.active_doc.and_then(|id| s.docs.get(id)).ok_or_else(|| err("No editor is active."))?;
                let Some(p) = &doc.editor.popup else { return Ok(json!([])) };
                let n = opt_usize(inv, "limit").unwrap_or(20);
                Ok(json!(p.shown.iter().take(n).map(|i| {
                    let it = &p.items[*i];
                    json!({ "label": it.label, "detail": it.detail, "kind": it.kind.tag(), "extra": it.label_detail })
                }).collect::<Vec<_>>()))
            },
        ),
        (
            CommandSpec::new("editor.accept_completion", "Accept Completion", Access::Edit)
                .description("Accepts an item from the completion popup (index into editor.completions, default 0), exactly like pressing Enter on it.")
                .params(json!({ "type": "object", "properties": { "index": { "type": "integer", "default": 0 } } })),
            |s, inv| {
                let id = s.active_doc.ok_or_else(|| err("No editor is active."))?;
                let doc = s.docs.get_mut(id).ok_or_else(|| err("No editor is active."))?;
                let requests = doc
                    .editor
                    .accept_completion_at(opt_usize(inv, "index").unwrap_or(0), &mut s.recency)
                    .ok_or_else(|| err("No completion popup, or no such item."))?;
                let edits = doc.editor.take_lsp_edits();
                s.lsp.did_change(id, &s.docs, &edits);
                for r in &requests {
                    s.lsp.request(id, &s.docs, r);
                }
                Ok(json!({ "accepted": true }))
            },
        ),
        (
            CommandSpec::new("editor.goto_line", "Go to Line", Access::Read)
                .description("Moves the active editor's cursor to a 1-based line (and column) and scrolls there.")
                .params(json!({ "type": "object", "required": ["line"], "properties": {
                    "line": { "type": "integer", "minimum": 1 }, "column": { "type": "integer", "minimum": 1 }
                }})),
            |s, inv| {
                let line = opt_usize(inv, "line").ok_or_else(|| CommandError::InvalidArgs("`line` is required".into()))?;
                let doc = s.active_doc.and_then(|id| s.docs.get_mut(id)).ok_or_else(|| err("No editor is active."))?;
                doc.editor.goto(line.saturating_sub(1), opt_usize(inv, "column").unwrap_or(1).saturating_sub(1));
                doc.editor.focus_requested = true;
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("editor.references", "Find All References", Access::Read)
                .description("Finds every use of the symbol at the cursor. Results appear in the References panel a moment later; read them with editor.reference_list."),
            |s, _| {
                let (id, _, head) = active(s)?;
                lsp_request(s, id, Request::References { at: head })?;
                Ok(json!({ "requested": true }))
            },
        ),
        (
            CommandSpec::new("editor.reference_list", "Reference List", Access::Read)
                .description("Results of the last Find All References: file, line, column, and the line's text."),
            |s, _| {
                Ok(json!(s.references.iter().map(|(p, l, c, t)| json!({ "path": rel(s, p), "line": l, "column": c, "text": t })).collect::<Vec<_>>()))
            },
        ),
        (CommandSpec::new("editor.rename_prompt", "Rename Symbol…", Access::Edit).ui_only(), |s, _| {
            let (id, _, head) = active(s)?;
            let doc = s.docs.get(id).expect("active");
            let (a, b) = perox_editor::core::word_at(&doc.editor.core.doc, head);
            s.rename = Some(doc.editor.core.doc.slice(a..b));
            Ok(Value::Null)
        }),
        (
            CommandSpec::new("editor.rename", "Rename Symbol", Access::Edit)
                .description("Renames the symbol at the active editor's cursor everywhere it's used (all files). Changes are unsaved and undoable.")
                .params(json!({ "type": "object", "required": ["new_name"], "properties": { "new_name": { "type": "string" } } })),
            |s, inv| {
                let (id, _, head) = active(s)?;
                let new_name = inv.str_arg("new_name")?.trim().to_owned();
                if new_name.is_empty() {
                    return Err(CommandError::InvalidArgs("`new_name` is empty".into()));
                }
                lsp_request(s, id, Request::Rename { at: head, new_name })?;
                Ok(json!({ "requested": true }))
            },
        ),
        (
            CommandSpec::new("editor.code_actions", "Quick Fix…", Access::Read)
                .description("Asks for quick fixes and refactorings at the cursor or selection. They appear as a menu a moment later; list them with editor.code_action_list and apply one with editor.apply_code_action."),
            |s, _| {
                let (id, range, _) = active(s)?;
                lsp_request(s, id, Request::CodeActions { range })?;
                Ok(json!({ "requested": true }))
            },
        ),
        (
            CommandSpec::new("editor.code_action_list", "Code Action List", Access::Read)
                .description("The quick fixes and refactorings currently offered (titles, by index)."),
            |s, _| Ok(json!(s.code_actions.as_ref().map(|(_, items)| items.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>()).unwrap_or_default())),
        ),
        (
            CommandSpec::new("editor.apply_code_action", "Apply Code Action", Access::Edit)
                .description("Applies a quick fix or refactoring from editor.code_action_list by index.")
                .params(json!({ "type": "object", "required": ["index"], "properties": { "index": { "type": "integer" } } })),
            |s, inv| {
                let i = opt_usize(inv, "index").ok_or_else(|| CommandError::InvalidArgs("`index` is required".into()))?;
                let (_, items) = s.code_actions.take().ok_or_else(|| err("No code actions are offered right now."))?;
                let (title, action) = items.into_iter().nth(i).ok_or_else(|| err("No such code action."))?;
                let mut effects = Vec::new();
                s.lsp.run_code_action(action, &mut effects);
                for e in effects {
                    match e {
                        crate::lsp::Effect::WorkspaceEdit(v) => {
                            apply_workspace_edit(s, &v);
                        }
                        crate::lsp::Effect::Message(m) => s.status = m,
                        _ => {}
                    }
                }
                Ok(json!({ "applied": title }))
            },
        ),
        (
            CommandSpec::new("editor.format", "Format Document", Access::Edit)
                .description("Formats the active file with rustfmt (through rust-analyzer). Undoable; not saved."),
            |s, _| {
                let (id, _, _) = active(s)?;
                lsp_request(s, id, Request::Format)?;
                Ok(json!({ "requested": true }))
            },
        ),
        (
            CommandSpec::new("view.toggle_intro", "Opening Animation", Access::Edit)
                .description("Turns the opening animation (shown when PeroxIDE starts) off or on."),
            |s, _| {
                s.settings.skip_intro = !s.settings.skip_intro;
                s.settings.save();
                s.status = format!("Opening animation {}", if s.settings.skip_intro { "off" } else { "on" });
                Ok(json!({ "opening_animation": !s.settings.skip_intro }))
            },
        ),
        (
            CommandSpec::new("editor.toggle_format_on_save", "Format on Save", Access::Edit)
                .description("Turns formatting Rust files on save on or off."),
            |s, _| {
                s.settings.format_on_save = !s.settings.format_on_save;
                s.settings.save();
                s.status = format!("Format on save {}", if s.settings.format_on_save { "on" } else { "off" });
                Ok(json!({ "format_on_save": s.settings.format_on_save }))
            },
        ),
        (
            CommandSpec::new("editor.cursor", "Cursor Position", Access::Read)
                .description("The active editor's file, cursor line and column (1-based) and selected text."),
            |s, _| {
                let doc = s.active_doc.and_then(|id| s.docs.get(id)).ok_or_else(|| err("No editor is active."))?;
                let sel = doc.editor.core.sels.primary();
                let p = doc.editor.core.doc.pos_of(sel.head);
                Ok(json!({
                    "path": rel(s, &doc.path), "line": p.line + 1, "column": p.col + 1,
                    "selection": doc.editor.core.doc.slice(sel.range())
                }))
            },
        ),
        (
            CommandSpec::new("file.save", "Save", Access::Edit)
                .description("Saves the active file, or the given one.")
                .params(json!({ "type": "object", "properties": { "path": { "type": "string" } } })),
            |s, inv| {
                let id = match inv.args.get("path") {
                    Some(_) => {
                        let p = resolve(s, inv, "path")?;
                        s.docs.by_path(&p).ok_or_else(|| err("That file isn't open."))?
                    }
                    None => s.active_doc.ok_or_else(|| err("No editor is active."))?,
                };
                let is_rust = s.docs.get(id).is_some_and(|d| d.path.extension().is_some_and(|e| e == "rs"));
                if s.settings.format_on_save && is_rust && s.lsp.format_then_save(id, &s.docs) {
                    return Ok(json!({ "saving": "after formatting" }));
                }
                let path = s.docs.save(id).map_err(err)?;
                s.lsp.did_save(id, &s.docs);
                    s.git.refresh(&s.ctx);
                s.status = format!("Saved {}", rel(s, &path));
                Ok(json!({ "saved": rel(s, &path) }))
            },
        ),
        (
            CommandSpec::new("file.save_all", "Save All", Access::Edit).description("Saves every file with unsaved changes."),
            |s, _| {
                let dirty: Vec<DocId> = s.docs.list.iter().filter(|d| d.is_dirty()).map(|d| d.id).collect();
                let mut saved = Vec::new();
                for id in dirty {
                    let p = s.docs.save(id).map_err(err)?;
                    s.lsp.did_save(id, &s.docs);
                    s.git.refresh(&s.ctx);
                    saved.push(rel(s, &p));
                }
                s.status = format!("Saved {} file(s)", saved.len());
                Ok(json!({ "saved": saved }))
            },
        ),
        (
            CommandSpec::new("file.close", "Close Editor", Access::Edit)
                .description("Closes the active editor, or the given file's. Refuses if there are unsaved changes unless `discard` is true.")
                .params(json!({ "type": "object", "properties": { "path": { "type": "string" }, "discard": { "type": "boolean" } } })),
            |s, inv| {
                let id = match inv.args.get("path") {
                    Some(_) => {
                        let p = resolve(s, inv, "path")?;
                        s.docs.by_path(&p).ok_or_else(|| err("That file isn't open."))?
                    }
                    None => s.active_doc.ok_or_else(|| err("No editor is active."))?,
                };
                close_doc(s, id, inv.bool_arg("discard")?.unwrap_or(false))
            },
        ),
        // ── build ──
        (build("build.check", "Check", "Runs `cargo check`: fast type checking without building."), |s, _| start_build(s, Kind::Check)),
        (build("build.build", "Build", "Runs `cargo build`."), |s, _| start_build(s, Kind::Build)),
        (build("build.run", "Run", "Runs `cargo run`: builds and starts the program. Its output appears in Output."), |s, _| start_build(s, Kind::Run)),
        (build("build.test", "Run Tests", "Runs `cargo test`; results also go to the Test Explorer (test.results)."), |s, _| crate::testing::run(s, crate::testing::Scope::All)),
        (build("build.clippy", "Clippy", "Runs `cargo clippy` for lints."), |s, _| start_build(s, Kind::Clippy)),
        (
            CommandSpec::new("build.snapshot", "Run and Screenshot", Access::Execute)
                .description("Runs the app (`cargo run`) and saves a PNG of its main window once it has settled (`delay_ms`, default 1500), then closes it: what the running app really looks like, data and all. Works with apps whose window is connected with perox-ui's generated `connect`. `out` (inside the project; default target/peroxide/screenshots/app.png); `args` are passed to the program. When build.status shows it finished, the PNG is there.")
                .params(json!({ "type": "object", "properties": {
                    "out": { "type": "string" },
                    "delay_ms": { "type": "integer", "minimum": 0 },
                    "args": { "type": "array", "items": { "type": "string" } }
                } })),
            |s, inv| {
                let out = match inv.args.get("out") {
                    Some(_) => resolve(s, inv, "out")?,
                    None => s
                        .project
                        .as_ref()
                        .map(|p| p.root.join("target/peroxide/screenshots/app.png"))
                        .ok_or_else(|| err("No project is open."))?,
                };
                let _ = std::fs::remove_file(&out);
                let mut env = vec![("PEROX_SNAPSHOT".to_owned(), out.to_string_lossy().into_owned())];
                if let Some(d) = inv.args.get("delay_ms").and_then(Value::as_u64) {
                    env.push(("PEROX_SNAPSHOT_DELAY".into(), d.to_string()));
                }
                let mut args = Vec::new();
                if let Some(a) = inv.args.get("args").and_then(Value::as_array) {
                    args.push("--".to_owned());
                    args.extend(a.iter().filter_map(|v| v.as_str().map(str::to_owned)));
                }
                let mut r = start_build_env(s, Kind::Run, &args, &env)?;
                r["out"] = json!(rel(s, &out));
                Ok(r)
            },
        ),
        (
            CommandSpec::new("build.package", "Package App…", Access::Execute)
                .description("Makes installable packages of the app: a release build (`cargo build --release`), then, by default, what this OS makes: a .app and .dmg on macOS, a .deb (and an AppImage if appimagetool is installed) on Linux, an .msi (if WiX is installed) and a .zip on Windows. Into target/peroxide/package. Name, identifier, publisher and icon come from [package.metadata.peroxide] in Cargo.toml. Signs and notarizes on macOS when PEROX_MACOS_SIGN_IDENTITY / PEROX_MACOS_NOTARY_PROFILE are set.")
                .params(json!({ "type": "object", "properties": {
                    "formats": { "type": "array", "items": { "type": "string", "enum": ["app", "dmg", "deb", "appimage", "msi", "zip"] } },
                    "path": { "type": "string", "description": "The app's folder, in a workspace (default: the project, else the package of the file being edited)" }
                } })),
            |s, inv| {
                let formats = match inv.args.get("formats").and_then(Value::as_array) {
                    Some(a) => a
                        .iter()
                        .map(|f| {
                            f.as_str()
                                .and_then(perox_package::Format::parse)
                                .ok_or_else(|| err(format!("Unknown package format {f}")))
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    None => perox_package::host_formats(),
                };
                let app = match inv.args.get("path") {
                    Some(_) => Some(resolve(s, inv, "path")?),
                    None => None,
                };
                start_package(s, app, formats)
            },
        ),
        (
            build("build.clippy_fix", "Apply Clippy Fixes", "Saves all files and runs `cargo clippy --fix`, which applies the lints' machine-applicable suggestions to the files. Review the result with git.diff."),
            |s, _| {
                let args: Vec<String> = ["--fix", "--allow-dirty", "--allow-staged"].map(String::from).to_vec();
                start_build_with(s, Kind::Clippy, &args)
            },
        ),
        (
            build("build.fmt", "Format Project", "Saves all files and runs `cargo fmt --all` (rustfmt on every file); open files reload with the result."),
            |s, _| start_build_with(s, Kind::Fmt, &["--all".into()]),
        ),
        (
            build("build.fmt_check", "Check Formatting", "Runs `cargo fmt --all -- --check`: lists the files rustfmt would change, with the diff, and changes nothing."),
            |s, _| start_build_with(s, Kind::Fmt, &["--all".into(), "--".into(), "--check".into()]),
        ),
        (
            CommandSpec::new("editor.toggle_clippy_on_save", "Clippy on Save", Access::Edit)
                .description("Switches what rust-analyzer runs on save between clippy (lints, with quick fixes) and plain cargo check (faster). Restarts rust-analyzer."),
            |s, _| {
                s.settings.plain_check_on_save = !s.settings.plain_check_on_save;
                s.settings.save();
                s.lsp.check_command = if s.settings.plain_check_on_save { "check" } else { "clippy" };
                if let Some(root) = s.project.as_ref().map(|p| p.root.clone()) {
                    s.lsp.start(&root);
                }
                s.status = format!("On save: cargo {}", s.lsp.check_command);
                Ok(json!({ "on_save": s.lsp.check_command }))
            },
        ),
        (build("build.stop", "Stop", "Stops the running cargo command and the program it started."), |s, _| {
            match &mut s.build.job {
                Some(job) => {
                    job.stop();
                    Ok(json!({ "stopped": true }))
                }
                None => Ok(json!({ "stopped": false, "note": "nothing was running" })),
            }
        }),
        (
            CommandSpec::new("build.status", "Build Status", Access::Read)
                .description("Whether a cargo command is running, and how the last one ended."),
            |s, _| {
                Ok(json!({
                    "running": s.build.running().map(Kind::verb),
                    "last": s.build.last.map(|(k, ok)| json!({ "command": k.verb(), "success": ok })),
                    "packaging": s.build.packaging.is_some() || s.build.package_after.is_some(),
                    "packaged": s.build.packaged.as_ref().map(|r| match r {
                        Ok(files) => json!({ "files": files.iter().map(|f| rel(s, f)).collect::<Vec<_>>() }),
                        Err(e) => json!({ "error": e }),
                    }),
                    "errors": s.build.diagnostics.iter().filter(|d| d.level == perox_build::Level::Error).count(),
                    "warnings": s.build.diagnostics.iter().filter(|d| d.level == perox_build::Level::Warning).count()
                }))
            },
        ),
        (
            CommandSpec::new("build.output", "Build Output", Access::Read)
                .description("The last lines of the Output panel: cargo's messages and the program's output.")
                .params(json!({ "type": "object", "properties": { "lines": { "type": "integer", "default": 100 } } })),
            |s, inv| {
                let n = opt_usize(inv, "lines").unwrap_or(100);
                let out = &s.build.output;
                Ok(json!(out[out.len().saturating_sub(n)..].join("\n")))
            },
        ),
        (
            CommandSpec::new("problems.list", "List Problems", Access::Read)
                .description("Errors and warnings: from rust-analyzer (live, as you type) and from the last cargo command."),
            |s, _| {
                let mut out = Vec::new();
                for p in s.lsp.diagnostics.values().flatten() {
                    out.push(json!({
                        "path": rel(s, &p.path), "line": p.line + 1, "column": p.col + 1,
                        "severity": format!("{:?}", p.severity).to_lowercase(), "message": p.message, "source": p.source
                    }));
                }
                for d in &s.build.diagnostics {
                    out.push(json!({
                        "path": d.file.as_ref().map(|f| rel(s, f)), "line": d.line, "column": d.column,
                        "severity": format!("{:?}", d.level).to_lowercase(), "message": d.message, "source": "cargo"
                    }));
                }
                Ok(json!(out))
            },
        ),
        (
            CommandSpec::new("lsp.status", "rust-analyzer Status", Access::Read)
                .description("What rust-analyzer is doing, and recent completion response times in milliseconds."),
            |s, _| {
                let ms = &s.lsp.completion_ms;
                let mut sorted = ms.clone();
                sorted.sort_by(f32::total_cmp);
                let pct = |p: f32| sorted.get(((sorted.len() as f32 - 1.0) * p).round() as usize).copied();
                Ok(json!({
                    "status": s.lsp.status, "busy": s.lsp.busy,
                    "completion_ms": { "count": ms.len(), "median": pct(0.5), "p90": pct(0.9), "max": sorted.last() }
                }))
            },
        ),
        (
            CommandSpec::new("terminal.show", "Terminal", Access::Read).ui_only(),
            |s, _| {
                terminal(s)?;
                s.layout_requests.push(LayoutRequest::ShowTerminal);
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("terminal.run", "Run in Terminal", Access::Execute)
                .description("Types a command line into the integrated terminal (a shell in the project folder) and presses Enter. Read the result with terminal.read after a moment.")
                .params(json!({ "type": "object", "required": ["command"], "properties": { "command": { "type": "string" } } })),
            |s, inv| {
                let command = inv.str_arg("command")?.replace('\n', " ");
                terminal(s)?.write(format!("{command}\r").into_bytes());
                s.layout_requests.push(LayoutRequest::ShowTerminal);
                Ok(json!({ "sent": command }))
            },
        ),
        (
            CommandSpec::new("terminal.read", "Read Terminal", Access::Read)
                .description("The text currently visible in the integrated terminal."),
            |s, _| {
                let t = s.terminal.as_ref().ok_or_else(|| err("The terminal isn't open."))?;
                Ok(json!(t.screen_text()))
            },
        ),
        (
            CommandSpec::new("view.source_control", "Source Control", Access::Read).ui_only(),
            |s, _| {
                s.git.refresh(&s.ctx);
                s.layout_requests.push(LayoutRequest::ShowSourceControl);
                Ok(Value::Null)
            },
        ),
        (
            CommandSpec::new("lsp.log", "rust-analyzer Log", Access::Read)
                .description("The last lines rust-analyzer wrote to its log, and failed requests.")
                .params(json!({ "type": "object", "properties": { "lines": { "type": "integer", "default": 50 } } })),
            |s, inv| {
                let n = opt_usize(inv, "lines").unwrap_or(50);
                let log = &s.lsp.log;
                Ok(json!(log[log.len().saturating_sub(n)..].join("\n")))
            },
        ),
        (
            CommandSpec::new("lsp.restart", "Restart rust-analyzer", Access::Execute).description("Restarts the language server."),
            |s, _| {
                let root = s.project.as_ref().map(|p| p.root.clone()).ok_or_else(|| err("No project is open."))?;
                s.lsp.start(&root);
                Ok(Value::Null)
            },
        ),
    ];
    for (spec, h) in commands {
        bus.register(spec, h)
            .expect("ide command ids are valid and unique");
    }
}

/// The active document and its primary cursor (or selection).
fn active(s: &AppState) -> Result<(DocId, std::ops::Range<usize>, usize), CommandError> {
    let id = s.active_doc.ok_or_else(|| err("No editor is active."))?;
    let doc = s.docs.get(id).ok_or_else(|| err("No editor is active."))?;
    let sel = doc.editor.core.sels.primary();
    Ok((id, sel.range(), sel.head))
}

fn lsp_request(s: &mut AppState, id: DocId, req: Request) -> Result<(), CommandError> {
    if !s.lsp.status.contains("ready") && !s.lsp.status.contains("rust-analyzer") {
        return Err(err("rust-analyzer isn't running for this project."));
    }
    s.lsp.request(id, &s.docs, &req);
    Ok(())
}

/// Applies a WorkspaceEdit: opens each file it touches and edits it (undoable, unsaved).
/// Returns how many files changed.
pub fn apply_workspace_edit(s: &mut AppState, edit: &Value) -> usize {
    let mut per_file: Vec<(PathBuf, Value)> = Vec::new();
    if let Some(changes) = edit["changes"].as_object() {
        for (uri, edits) in changes {
            if let Some(p) = perox_lsp::uri::to_path(uri) {
                per_file.push((p, edits.clone()));
            }
        }
    }
    for dc in edit["documentChanges"].as_array().into_iter().flatten() {
        if let Some(p) = dc["textDocument"]["uri"]
            .as_str()
            .and_then(perox_lsp::uri::to_path)
        {
            per_file.push((p, dc["edits"].clone()));
        }
    }
    let mut n = 0;
    for (path, edits) in per_file {
        let Ok((id, new)) = s.docs.open(&path) else {
            continue;
        };
        if new {
            s.lsp.did_open(id, &s.docs);
            s.layout_requests.push(LayoutRequest::ShowDoc(id));
        }
        let Some(doc) = s.docs.get_mut(id) else {
            continue;
        };
        let changes: Vec<Change> = edits
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| {
                let at = |p: &Value| {
                    doc.editor.core.doc.char_of(Pos {
                        line: p["line"].as_u64().unwrap_or(0) as usize,
                        col: p["character"].as_u64().unwrap_or(0) as usize,
                    })
                };
                let (a, b) = (at(&e["range"]["start"]), at(&e["range"]["end"]));
                Change::replace(a..b.max(a), e["newText"].as_str().unwrap_or_default())
            })
            .collect();
        doc.editor.apply_extra_edits(changes);
        let lsp_edits = doc.editor.take_lsp_edits();
        s.lsp.did_change(id, &s.docs, &lsp_edits);
        n += 1;
    }
    n
}

/// Break Layout / Lay Out from an agent, using where the designer last drew things.
fn relayout_command(
    s: &mut AppState,
    inv: &Invocation,
    dir: Option<perox_designer::free::Direction>,
) -> Result<Value, CommandError> {
    let (id, text) = form_text(s, inv)?;
    let parsed = perox_slint::parse(&text);
    let comp = component_name(&parsed, inv)?;
    let path = widget_path(&parsed, &comp, inv.str_arg("widget")?)?;
    let doc = s
        .docs
        .get_mut(id)
        .ok_or_else(|| err("The form isn't open."))?;
    let designer = doc
        .designer
        .as_ref()
        .filter(|d| doc.design_view && d.component_name() == comp)
        .ok_or_else(|| err("Show the component in the designer first (designer.select)."))?;
    let drawn = perox_designer::free::Drawn {
        rects: &designer.layout.elements,
    };
    if drawn.rects.len() < 2 {
        return Err(err("The designer hasn't drawn this form yet; try again."));
    }
    let result = match dir {
        None => perox_designer::free::break_layout(&text, &comp, drawn, &path),
        Some(d) => perox_designer::free::lay_out(&text, &comp, drawn, &path, d),
    }
    .map_err(|e| err(e.to_string()))?;
    let edits = perox_slint::edit::diff(&text, &result);
    apply_form_edits(s, id, edits);
    Ok(json!({ "ok": true }))
}

/// Opens a `.slint` file (keeping the current view) and returns it with its text.
/// `to` relative to the folder `from`, with `/` and `..`.
fn pathdiff_rel(to: &std::path::Path, from: &std::path::Path) -> String {
    let to: Vec<_> = to.components().collect();
    let from: Vec<_> = from.components().collect();
    let common = to.iter().zip(&from).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> =
        std::iter::repeat_n("..".to_owned(), from.len() - common).collect();
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// The design database of the form at `path` (data.database).
fn design_db(s: &mut AppState, inv: &Invocation) -> Result<perox_ui::data::Database, CommandError> {
    let path = resolve(s, inv, "path")?;
    let dir = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let db = perox_slint::project::settings_for(&dir)
        .database
        .ok_or_else(|| err("No design database: data.database first."))?;
    let root = perox_designer::data::crate_root(&dir)
        .ok_or_else(|| err("No Cargo.toml above the form."))?;
    perox_ui::data::Database::open(&format!("sqlite:{}", root.join(db).display()))
        .map_err(|e| err(e.to_string()))
}

fn form_text(s: &mut AppState, inv: &Invocation) -> Result<(DocId, String), CommandError> {
    let path = resolve(s, inv, "path")?;
    if !crate::docs::is_form(&path) {
        return Err(err("Not a .slint file."));
    }
    let id = open_file(s, &path, None, None)?;
    let text = s
        .docs
        .get(id)
        .map(|d| d.editor.core.text())
        .unwrap_or_default();
    Ok((id, text))
}

/// The `component` argument, else the file's main component.
fn component_name(doc: &perox_slint::Document, inv: &Invocation) -> Result<String, CommandError> {
    match inv.args["component"].as_str() {
        Some(c) if doc.component(c).is_some() => Ok(c.to_owned()),
        Some(c) => Err(err(format!("No component `{c}` in this file."))),
        None => doc
            .main_component()
            .map(|i| doc.components[i].name.clone())
            .ok_or_else(|| err("This file has no component.")),
    }
}

fn s_slice(text: &str, r: &std::ops::Range<usize>) -> String {
    text.get(r.clone()).unwrap_or_default().to_owned()
}

fn apply_form_edits(s: &mut AppState, id: DocId, edits: Vec<perox_slint::edit::TextEdit>) {
    if let Some(doc) = s.docs.get_mut(id) {
        let edits: Vec<_> = edits.into_iter().map(|e| (e.range, e.text)).collect();
        doc.editor.apply_byte_edits(&edits);
    }
}

/// `#0.1` for a path; `#` is the root.
fn path_key(path: &[usize]) -> String {
    let parts: Vec<String> = path.iter().map(usize::to_string).collect();
    format!("#{}", parts.join("."))
}

/// An element id or `#0.1` path.
fn widget_path(
    doc: &perox_slint::Document,
    comp: &str,
    w: &str,
) -> Result<Vec<usize>, CommandError> {
    let c = doc
        .component(comp)
        .ok_or_else(|| err(format!("No component `{comp}`.")))?;
    let path = if let Some(rest) = w.strip_prefix('#') {
        rest.split('.')
            .filter(|p| !p.is_empty())
            .map(|p| {
                p.parse::<usize>()
                    .map_err(|_| err(format!("`{w}` isn't a path like #0.1")))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        c.find(w)
            .ok_or_else(|| err(format!("No element named `{w}`.")))?
    };
    c.at(&path)
        .ok_or_else(|| err(format!("Nothing at `{w}`.")))?;
    Ok(path)
}

fn form_json(el: &perox_slint::Element, path: &mut Vec<usize>) -> Value {
    let props: serde_json::Map<String, Value> = el
        .props
        .iter()
        .map(|p| (p.name.clone(), Value::String(p.value.to_source())))
        .collect();
    let handlers: serde_json::Map<String, Value> = el
        .handlers
        .iter()
        .map(|h| (h.event.clone(), Value::String(h.body.clone())))
        .collect();
    let mut children = Vec::new();
    for (i, c) in el.children.iter().enumerate() {
        path.push(i);
        children.push(form_json(c, path));
        path.pop();
    }
    json!({
        "path": path_key(path),
        "kind": el.kind,
        "id": el.id,
        "props": props,
        "handlers": handlers,
        "children": children,
    })
}

/// Finds `impl <form>Handlers for …` in the project, opens it at `method`, adding the
/// method if needed.
fn open_handler(
    s: &mut AppState,
    form: &str,
    method: &str,
    params: &[(String, String)],
) -> Result<Value, CommandError> {
    let trait_name = format!("{form}Handlers");
    let text_of = |s: &AppState, p: &Path| match s.docs.by_path(p).and_then(|id| s.docs.get(id)) {
        Some(d) => Some(d.editor.core.text()),
        None => std::fs::read_to_string(p).ok(),
    };
    let root = s
        .project
        .as_ref()
        .map(|p| p.root.clone())
        .ok_or_else(|| err("No project is open."))?;
    // `files` is relative to the project root.
    let found = s
        .files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| root.join(p))
        .find_map(|p| {
            let text = text_of(s, &p)?;
            let body = crate::code_behind::find_impl(&text, &trait_name)?;
            Some((p, text, body))
        });
    let Some((path, text, body)) = found else {
        return Err(err(format!(
            "No `impl {trait_name} for …` in this project. Add one to your code (see examples/word-counter), then try again."
        )));
    };
    let line_of = |t: &str, off: usize| t[..off].matches('\n').count() + 1;
    if let Some(at) = crate::code_behind::find_method(&text, body.clone(), method) {
        open_file(s, &path, Some(line_of(&text, at)), None)?;
        return Ok(json!({ "path": rel(s, &path), "line": line_of(&text, at), "added": false }));
    }
    let (at, insert, cursor) = crate::code_behind::new_method(&text, body, form, method, params);
    let id = open_file(s, &path, None, None)?;
    if let Some(doc) = s.docs.get_mut(id) {
        doc.editor.apply_byte_edits(&[(at..at, insert.clone())]);
        let target = at + cursor;
        let mut new_text = text.clone();
        new_text.insert_str(at, &insert);
        let line = line_of(&new_text, target);
        let col = new_text[..target]
            .rsplit('\n')
            .next()
            .map_or(0, |l| l.chars().count());
        doc.editor.goto(line - 1, col);
        doc.editor.focus_requested = true;
        return Ok(json!({ "path": rel(s, &path), "line": line, "added": true }));
    }
    Ok(json!({ "path": rel(s, &path), "added": true }))
}

/// The terminal, started in the project folder (or home) on first use.
fn terminal(s: &mut AppState) -> Result<&mut perox_terminal::Terminal, CommandError> {
    if s.terminal.as_ref().is_some_and(|t| t.exited) {
        s.terminal = None;
    }
    if s.terminal.is_none() {
        let cwd = s
            .project
            .as_ref()
            .map(|p| p.root.clone())
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let t = perox_terminal::Terminal::spawn(&cwd, &perox_build::tools::env(), s.ctx.clone())
            .map_err(|e| err(format!("Couldn't start a shell: {e}")))?;
        s.terminal = Some(t);
    }
    Ok(s.terminal.as_mut().expect("just set"))
}

pub fn close_doc(s: &mut AppState, id: DocId, discard: bool) -> Result<Value, CommandError> {
    let doc = s.docs.get(id).ok_or_else(|| err("No such editor."))?;
    if doc.is_dirty() && !discard {
        return Err(err(format!(
            "{} has unsaved changes. Save it first, or close with discard.",
            rel(s, &doc.path)
        )));
    }
    let path = doc.path.clone();
    s.docs.close(id);
    s.lsp.did_close(id, &path);
    if s.active_doc == Some(id) {
        s.active_doc = s.docs.list.last().map(|d| d.id);
    }
    s.layout_requests.push(LayoutRequest::CloseDoc(id));
    Ok(json!({ "closed": rel(s, &path) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_dot_dot() {
        assert_eq!(
            normalize(Path::new("/p/src/../../etc/passwd")),
            PathBuf::from("/etc/passwd")
        );
        assert!(!normalize(Path::new("/p/src/../../etc")).starts_with("/p"));
        assert!(normalize(Path::new("/p/./src/x.rs")).starts_with("/p"));
    }
}
