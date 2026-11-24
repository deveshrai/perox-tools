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
}

/// The current cargo job and what it printed.
#[derive(Debug, Default)]
pub struct BuildState {
    pub job: Option<Job>,
    pub output: Vec<String>,
    pub diagnostics: Vec<perox_build::Diagnostic>,
    /// The last finished job and whether it succeeded.
    pub last: Option<(Kind, bool)>,
}

impl BuildState {
    pub fn running(&self) -> Option<Kind> {
        self.job.as_ref().map(|j| j.kind)
    }

    /// Collects new output; returns `Some(success)` when the job finished this frame.
    pub fn poll(&mut self) -> Option<(Kind, bool)> {
        let job = self.job.as_mut()?;
        let kind = job.kind;
        let mut finished = None;
        for e in job.poll() {
            match e {
                perox_build::Event::Line(l) => {
                    if self.output.len() >= 20_000 {
                        self.output.drain(..5_000);
                    }
                    self.output.push(l);
                }
                perox_build::Event::Diagnostic(d) => self.diagnostics.push(d),
                perox_build::Event::Finished { success } => finished = Some(success),
            }
        }
        let success = finished?;
        self.job = None;
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
fn resolve(s: &AppState, inv: &Invocation, name: &str) -> Result<PathBuf, CommandError> {
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
    let ctx = s.ctx.clone();
    let job = Job::start(&cargo, &root, kind, move || ctx.request_repaint())
        .map_err(|e| err(e.to_string()))?;
    s.build.job = Some(job);
    s.build.output.clear();
    s.build.diagnostics.clear();
    s.build.output.push(format!("── cargo {} ──", kind.verb()));
    s.layout_requests.push(LayoutRequest::ShowOutput);
    s.status = format!("cargo {}…", kind.verb());
    Ok(
        json!({ "started": format!("cargo {}", kind.verb()), "hint": "Call build.output (or problems.list) for results; build.status says when it's done." }),
    )
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
                .description("Adds an element to a component (in the editor, unsaved), importing it from std-widgets.slint if needed. Kinds: VerticalBox HorizontalBox GridBox Rectangle GroupBox TabWidget Tab ScrollView Text Button LineEdit TextEdit CheckBox Switch ComboBox SpinBox Slider ProgressIndicator Spinner StandardListView Image TouchArea, or any component. Property values are .slint expressions: \"\\\"Hello\\\"\", \"12px\", \"true\", \"#ff8800\", \"[\\\"a\\\", \\\"b\\\"]\".")
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
                let parent = widget_path(&doc, &comp, inv.str_arg("parent")?)?;
                let index = opt_usize(inv, "index").unwrap_or(usize::MAX);
                let mut props = Vec::new();
                for (k, v) in inv.args["props"].as_object().into_iter().flatten() {
                    let src = v.as_str().ok_or_else(|| err(format!("`{k}` must be a string with a .slint expression")))?;
                    props.push((k.clone(), src.to_owned()));
                }
                let kind = inv.str_arg("kind")?;
                // Like the designer: widgets code talks to get an id.
                let auto = doc
                    .component(&comp)
                    .filter(|_| perox_designer::catalog::wants_id(kind))
                    .map(|c| perox_designer::unique_id(c, &perox_designer::catalog::snake(kind)));
                let wid = inv.args["id"].as_str().or(auto.as_deref());
                let mut work = text.clone();
                perox_designer::free::step(&mut work, perox_slint::edit::add_element(&text, &comp, &parent, index, kind, wid, &props))
                    .map_err(|e| err(e.to_string()))?;
                perox_designer::free::import(&mut work, kind);
                apply_form_edits(s, id, perox_slint::edit::diff(&text, &work));
                let count = doc.component(&comp).and_then(|c| c.at(&parent)).map_or(0, |p| p.children.len());
                let mut path = parent;
                path.push(index.min(count));
                Ok(json!({ "added": path_key(&path), "id": wid }))
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
                apply_form_edits(s, id, edits);
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
                .description("Wires an element's callback (clicked, edited, accepted, toggled, changed, selected, …) to a root callback `method` (declared if missing, and so a method of the generated <Component>Handlers trait), or unwires it when method is null.")
                .params(path_schema(json!({
                    "component": { "type": "string" },
                    "widget": json!({ "type": "string", "description": "Element id, or a path like #0.1 (# is the component root)" }),
                    "event": { "type": "string" },
                    "method": { "type": ["string", "null"] }
                }))),
            |s, inv| {
                let (id, text) = form_text(s, inv)?;
                let doc = perox_slint::parse(&text);
                let comp = component_name(&doc, inv)?;
                let path = widget_path(&doc, &comp, inv.str_arg("widget")?)?;
                let event = inv.str_arg("event")?;
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
        (build("build.test", "Run Tests", "Runs `cargo test`."), |s, _| start_build(s, Kind::Test)),
        (build("build.clippy", "Clippy", "Runs `cargo clippy` for lints."), |s, _| start_build(s, Kind::Clippy)),
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
            CommandSpec::new("git.status", "Git Status", Access::Read)
                .description("The project's git branch, commits ahead/behind, and changed files with their state (modified, added, deleted, renamed, untracked, conflicted)."),
            |s, _| {
                if s.git.root.is_none() {
                    return Err(err("The project isn't in a git repository (or git isn't installed)."));
                }
                let st = &s.git.status;
                Ok(json!({
                    "branch": st.branch, "ahead": st.ahead, "behind": st.behind,
                    "files": st.files.iter().map(|(p, f)| json!({ "path": rel(s, p), "state": format!("{f:?}").to_lowercase() })).collect::<Vec<_>>()
                }))
            },
        ),
        (
            CommandSpec::new("git.diff", "Git Diff", Access::Read)
                .description("Unified diff of the working tree against the last commit: one file, or everything. Only saved changes count.")
                .params(json!({ "type": "object", "properties": { "path": { "type": "string" } } })),
            |s, inv| {
                let path = match inv.args.get("path") {
                    Some(_) => Some(resolve(s, inv, "path")?),
                    None => None,
                };
                Ok(json!(s.git.diff(path.as_deref()).map_err(err)?))
            },
        ),
        (
            CommandSpec::new("git.commit", "Commit All Changes", Access::Execute)
                .description("Saves all files, stages every change and commits with the message.")
                .params(json!({ "type": "object", "required": ["message"], "properties": { "message": { "type": "string" } } })),
            |s, inv| {
                let message = inv.str_arg("message")?.to_owned();
                let dirty: Vec<DocId> = s.docs.list.iter().filter(|d| d.is_dirty()).map(|d| d.id).collect();
                for id in dirty {
                    s.docs.save(id).map_err(err)?;
                    s.lsp.did_save(id, &s.docs);
                }
                let summary = s.git.commit(&message, true).map_err(err)?;
                s.commit_message.clear();
                s.git.refresh(&s.ctx);
                s.status = summary.clone();
                Ok(json!({ "committed": summary }))
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
