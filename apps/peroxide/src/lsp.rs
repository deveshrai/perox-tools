//! The app's side of rust-analyzer: starts it for the open project, keeps open
//! documents in sync, sends the editor's requests and turns answers into editor state
//! (completion popup, hover, signature, diagnostics, semantic colours).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use perox_editor::completion::{Item, Kind, Popup, Recency};
use perox_editor::syntax::Hl;
use perox_editor::{Change, Diagnostic, Edited, EditorState, Pos, Request, Severity, Signature};
use perox_lsp::{Event, RustAnalyzer, uri};
use serde_json::{Value, json};

use crate::docs::{DocId, Docs};

type Started = Result<(RustAnalyzer, Receiver<Event>), String>;

enum State {
    Off,
    Starting(Receiver<Started>),
    Running {
        ra: RustAnalyzer,
        events: Receiver<Event>,
    },
    Failed,
}

#[derive(Debug)]
enum Pending {
    Completion {
        doc: DocId,
        at: usize,
        sent: Instant,
    },
    Resolve {
        doc: DocId,
    },
    Hover {
        doc: DocId,
        at: usize,
    },
    Definition,
    Signature {
        doc: DocId,
    },
    Semantic {
        doc: DocId,
        version: i32,
    },
}

/// A diagnostic for the Problems panel (0-based line, char column).
#[derive(Debug, Clone)]
pub struct Problem {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub severity: Severity,
    pub message: String,
    pub source: String,
}

/// Something the app must do because of an answer.
#[derive(Debug)]
pub enum Effect {
    Open {
        path: PathBuf,
        line: usize,
        col: usize,
    },
}

pub struct Lsp {
    state: State,
    pending: HashMap<i64, Pending>,
    /// Open documents and the version last sent.
    opened: HashMap<DocId, i32>,
    /// Raw items of the latest completion answer, for `completionItem/resolve`.
    raw_items: Vec<Value>,
    completion_req: Option<i64>,
    semantic_due: HashMap<DocId, Instant>,
    pub diagnostics: BTreeMap<PathBuf, Vec<Problem>>,
    /// What rust-analyzer is doing, for the status bar.
    pub status: String,
    pub busy: bool,
    pub log: Vec<String>,
    /// Work in progress ($/progress tokens) and its titles.
    progress: BTreeMap<String, String>,
    quiescent: bool,
    /// Recent completion round-trip times (ms), for the quality bar.
    pub completion_ms: Vec<f32>,
    ctx: egui::Context,
}

impl std::fmt::Debug for Lsp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lsp")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

fn severity(n: Option<u64>) -> Severity {
    match n {
        Some(1) => Severity::Error,
        Some(2) => Severity::Warning,
        Some(3) => Severity::Info,
        _ => Severity::Hint,
    }
}

/// LSP (line, character) → char index. With utf-32 a character is a char.
fn to_char(ed: &EditorState, pos: &Value, utf32: bool) -> usize {
    let line = pos["line"].as_u64().unwrap_or(0) as usize;
    let ch = pos["character"].as_u64().unwrap_or(0) as usize;
    let col = if utf32 {
        ch
    } else {
        utf16_to_col(&ed.core.doc.line_text(line), ch)
    };
    ed.core.doc.char_of(Pos { line, col })
}

fn utf16_to_col(line: &str, units: usize) -> usize {
    let mut n = 0;
    for (i, c) in line.chars().enumerate() {
        if n >= units {
            return i;
        }
        n += c.len_utf16();
    }
    line.chars().count()
}

fn to_lsp(ed: &EditorState, at: usize, utf32: bool) -> Value {
    let p = ed.core.doc.pos_of(at);
    let ch = if utf32 {
        p.col
    } else {
        ed.core
            .doc
            .line_text(p.line)
            .chars()
            .take(p.col)
            .map(char::len_utf16)
            .sum()
    };
    json!({ "line": p.line, "character": ch })
}

fn markup(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().filter_map(markup).collect();
            (!parts.is_empty()).then(|| parts.join("\n\n"))
        }
        _ => None,
    }
}

fn kind(n: Option<u64>, label: &str) -> Kind {
    if label.contains("!(") || label.contains("! {") || label.contains("![") {
        return Kind::Macro;
    }
    match n {
        Some(2) => Kind::Method,
        Some(3 | 4) => Kind::Function,
        Some(5 | 10) => Kind::Field,
        Some(6) => Kind::Variable,
        Some(7 | 22) => Kind::Struct,
        Some(8) => Kind::Trait,
        Some(9) => Kind::Module,
        Some(13) => Kind::Enum,
        Some(14) => Kind::Keyword,
        Some(15) => Kind::Snippet,
        Some(20) => Kind::EnumMember,
        Some(21) => Kind::Constant,
        Some(25) => Kind::TypeParam,
        _ => Kind::Other,
    }
}

impl Lsp {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            state: State::Off,
            pending: HashMap::new(),
            opened: HashMap::new(),
            raw_items: Vec::new(),
            completion_req: None,
            semantic_due: HashMap::new(),
            diagnostics: BTreeMap::new(),
            status: String::new(),
            busy: false,
            log: Vec::new(),
            progress: BTreeMap::new(),
            quiescent: false,
            completion_ms: Vec::new(),
            ctx,
        }
    }

    /// Starts rust-analyzer for a project, in the background.
    pub fn start(&mut self, root: &Path) {
        self.stop();
        let Some(program) = perox_build::tools::rust_analyzer() else {
            self.state = State::Failed;
            self.status = "rust-analyzer not found".into();
            return;
        };
        let (tx, rx) = mpsc::channel();
        let root = root.to_owned();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let wake_ctx = ctx.clone();
            let result = perox_lsp::rust_analyzer::start(&program, &root, move || {
                wake_ctx.request_repaint()
            });
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.state = State::Starting(rx);
        self.status = "rust-analyzer: starting".into();
        self.busy = true;
    }

    pub fn stop(&mut self) {
        if let State::Running { ra, .. } = std::mem::replace(&mut self.state, State::Off) {
            std::thread::spawn(move || ra.client.shutdown());
        }
        self.pending.clear();
        self.opened.clear();
        self.diagnostics.clear();
        self.status.clear();
        self.busy = false;
        self.progress.clear();
        self.quiescent = false;
    }

    fn ra(&self) -> Option<&RustAnalyzer> {
        match &self.state {
            State::Running { ra, .. } => Some(ra),
            _ => None,
        }
    }

    fn utf32(&self) -> bool {
        self.ra().is_none_or(|ra| ra.utf32)
    }

    // ── document sync ─────────────────────────────────────────────────────

    fn is_rust(path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "rs")
    }

    pub fn did_open(&mut self, id: DocId, docs: &Docs) {
        let Some(doc) = docs.get(id) else { return };
        let Some(ra) = self.ra() else { return };
        if !Self::is_rust(&doc.path) || self.opened.contains_key(&id) {
            return;
        }
        let version = doc.editor.core.doc.version();
        ra.client.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": uri::from_path(&doc.path), "languageId": "rust",
                "version": version, "text": doc.editor.core.text()
            }}),
        );
        self.opened.insert(id, version);
        self.semantic_due.insert(id, Instant::now());
    }

    pub fn did_change(&mut self, id: DocId, docs: &Docs, edits: &[Edited]) {
        if edits.is_empty() || !self.opened.contains_key(&id) {
            return;
        }
        let (Some(doc), Some(ra)) = (docs.get(id), self.ra()) else {
            return;
        };
        let version = doc.editor.core.doc.version();
        let changes: Vec<Value> = if ra.utf32 {
            edits
                .iter()
                .map(|e| {
                    json!({
                        "range": {
                            "start": { "line": e.start.line, "character": e.start.col },
                            "end": { "line": e.old_end.line, "character": e.old_end.col }
                        },
                        "text": e.new_text
                    })
                })
                .collect()
        } else {
            vec![json!({ "text": doc.editor.core.text() })]
        };
        ra.client.notify(
            "textDocument/didChange",
            json!({ "textDocument": { "uri": uri::from_path(&doc.path), "version": version }, "contentChanges": changes }),
        );
        self.opened.insert(id, version);
        self.semantic_due
            .insert(id, Instant::now() + Duration::from_millis(350));
    }

    pub fn did_save(&mut self, id: DocId, docs: &Docs) {
        if let (Some(doc), Some(ra)) = (docs.get(id), self.ra())
            && self.opened.contains_key(&id)
        {
            ra.client.notify(
                "textDocument/didSave",
                json!({ "textDocument": { "uri": uri::from_path(&doc.path) } }),
            );
        }
    }

    pub fn did_close(&mut self, id: DocId, path: &Path) {
        if self.opened.remove(&id).is_some()
            && let Some(ra) = self.ra()
        {
            ra.client.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": uri::from_path(path) } }),
            );
        }
    }

    // ── requests ──────────────────────────────────────────────────────────

    pub fn request(&mut self, id: DocId, docs: &Docs, req: &Request) {
        if !self.opened.contains_key(&id) {
            return;
        }
        let ra = match &self.state {
            State::Running { ra, .. } => ra,
            _ => return,
        };
        let Some(doc) = docs.get(id) else { return };
        let ed = &doc.editor;
        let utf32 = ra.utf32;
        let td = json!({ "uri": uri::from_path(&doc.path) });
        let (method, params, pending) = match req {
            Request::Completion { at, trigger } => {
                if let Some(old) = self.completion_req.take() {
                    ra.client.cancel(old);
                    self.pending.remove(&old);
                }
                let context = match trigger {
                    Some(c) => json!({ "triggerKind": 2, "triggerCharacter": c.to_string() }),
                    None => json!({ "triggerKind": 1 }),
                };
                (
                    "textDocument/completion",
                    json!({ "textDocument": td, "position": to_lsp(ed, *at, utf32), "context": context }),
                    Pending::Completion {
                        doc: id,
                        at: *at,
                        sent: Instant::now(),
                    },
                )
            }
            Request::ResolveCompletion { server_index } => {
                let Some(item) = self.raw_items.get(*server_index).cloned() else {
                    return;
                };
                ("completionItem/resolve", item, Pending::Resolve { doc: id })
            }
            Request::Hover { at } => (
                "textDocument/hover",
                json!({ "textDocument": td, "position": to_lsp(ed, *at, utf32) }),
                Pending::Hover { doc: id, at: *at },
            ),
            Request::Definition { at } => (
                "textDocument/definition",
                json!({ "textDocument": td, "position": to_lsp(ed, *at, utf32) }),
                Pending::Definition,
            ),
            Request::SignatureHelp { at } => (
                "textDocument/signatureHelp",
                json!({ "textDocument": td, "position": to_lsp(ed, *at, utf32) }),
                Pending::Signature { doc: id },
            ),
        };
        let rid = ra.client.request(method, params);
        if matches!(pending, Pending::Completion { .. }) {
            self.completion_req = Some(rid);
        }
        self.pending.insert(rid, pending);
    }

    // ── answers ───────────────────────────────────────────────────────────

    /// Call every frame: finishes startup, applies answers, sends due semantic requests.
    pub fn poll(&mut self, docs: &mut Docs, recency: &Recency) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let State::Starting(rx) = &self.state
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok((ra, events)) => {
                    self.state = State::Running { ra, events };
                    self.status = "rust-analyzer: loading the project".into();
                    let ids: Vec<DocId> = docs.list.iter().map(|d| d.id).collect();
                    for id in ids {
                        self.did_open(id, docs);
                    }
                }
                Err(e) => {
                    self.status = format!("rust-analyzer failed: {e}");
                    self.log.push(e);
                    self.busy = false;
                    self.state = State::Failed;
                }
            }
        }
        let events: Vec<Event> = match &self.state {
            State::Running { events, .. } => events.try_iter().collect(),
            _ => Vec::new(),
        };
        for event in events {
            match event {
                Event::Response { id, result } => {
                    if let Some(p) = self.pending.remove(&id)
                        && let Ok(v) = result
                    {
                        self.answer(p, v, docs, recency, &mut effects);
                    }
                }
                Event::Notification { method, params } => self.notification(&method, &params, docs),
                Event::Request { id, .. } => {
                    if let Some(ra) = self.ra() {
                        ra.client.respond(id, Value::Null);
                    }
                }
                Event::Stderr(line) => {
                    if self.log.len() > 2000 {
                        self.log.drain(..1000);
                    }
                    self.log.push(line);
                }
                Event::Exited => {
                    self.status = "rust-analyzer stopped".into();
                    self.busy = false;
                    self.state = State::Failed;
                }
            }
        }
        self.send_due_semantic(docs);
        effects
    }

    fn notification(&mut self, method: &str, params: &Value, docs: &mut Docs) {
        match method {
            "textDocument/publishDiagnostics" => {
                let Some(path) = params["uri"].as_str().and_then(uri::to_path) else {
                    return;
                };
                let utf32 = self.utf32();
                let problems: Vec<Problem> = params["diagnostics"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|d| {
                        let n = |v: &Value| v.as_u64().unwrap_or(0) as usize;
                        let r = &d["range"];
                        Problem {
                            path: path.clone(),
                            line: n(&r["start"]["line"]),
                            col: n(&r["start"]["character"]),
                            end_line: n(&r["end"]["line"]),
                            end_col: n(&r["end"]["character"]),
                            severity: severity(d["severity"].as_u64()),
                            message: d["message"].as_str().unwrap_or_default().to_owned(),
                            source: d["source"].as_str().unwrap_or("rust-analyzer").to_owned(),
                        }
                    })
                    .collect();
                if let Some(id) = docs.by_path(&path)
                    && let Some(doc) = docs.get_mut(id)
                {
                    let ed = &doc.editor;
                    let diags = problems
                        .iter()
                        .map(|p| {
                            let s =
                                to_char(ed, &json!({"line": p.line, "character": p.col}), utf32);
                            let e = to_char(
                                ed,
                                &json!({"line": p.end_line, "character": p.end_col}),
                                utf32,
                            );
                            Diagnostic {
                                range: s..e.max(s),
                                severity: p.severity,
                                message: p.message.clone(),
                                source: p.source.clone(),
                            }
                        })
                        .collect();
                    doc.editor.deco.diagnostics = diags;
                }
                if problems.is_empty() {
                    self.diagnostics.remove(&path);
                } else {
                    self.diagnostics.insert(path, problems);
                }
            }
            "experimental/serverStatus" => {
                self.quiescent = params["quiescent"].as_bool().unwrap_or(true);
                if params["health"].as_str() == Some("error")
                    && let Some(m) = params["message"].as_str()
                {
                    self.log.push(m.to_owned());
                }
                self.update_status();
            }
            "$/progress" => {
                let token = params["token"].to_string();
                let v = &params["value"];
                match v["kind"].as_str() {
                    Some("end") => {
                        self.progress.remove(&token);
                    }
                    Some("begin") => {
                        let title = v["title"].as_str().unwrap_or("working").to_owned();
                        self.progress.insert(token, title);
                    }
                    _ => {
                        if let (Some(t), Some(pct)) =
                            (self.progress.get_mut(&token), v["percentage"].as_u64())
                        {
                            let base = t.split(" — ").next().unwrap_or_default().to_owned();
                            *t = format!("{base} — {pct}%");
                        }
                    }
                }
                self.update_status();
            }
            _ => {}
        }
    }

    fn update_status(&mut self) {
        self.busy = !self.progress.is_empty() || !self.quiescent;
        self.status = match self.progress.values().next_back() {
            Some(t) => format!("rust-analyzer: {t}"),
            None if self.quiescent => "rust-analyzer: ready".into(),
            None => "rust-analyzer: loading".into(),
        };
    }

    fn answer(
        &mut self,
        p: Pending,
        v: Value,
        docs: &mut Docs,
        recency: &Recency,
        effects: &mut Vec<Effect>,
    ) {
        let utf32 = self.utf32();
        match p {
            Pending::Completion { doc, at, sent } => {
                self.completion_req = None;
                let ms = sent.elapsed().as_secs_f32() * 1000.0;
                if self.completion_ms.len() >= 50 {
                    self.completion_ms.remove(0);
                }
                self.completion_ms.push(ms);
                let Some(d) = docs.get_mut(doc) else { return };
                let (items, incomplete) = match &v {
                    Value::Array(a) => (a.clone(), false),
                    Value::Object(_) => (
                        v["items"].as_array().cloned().unwrap_or_default(),
                        v["isIncomplete"].as_bool().unwrap_or(false),
                    ),
                    _ => (Vec::new(), false),
                };
                let ed = &d.editor;
                let converted = items
                    .iter()
                    .enumerate()
                    .map(|(i, it)| convert_item(ed, it, i, at, utf32))
                    .collect();
                self.raw_items = items;
                d.editor
                    .set_completions(Popup::new(converted, at, incomplete), at, recency);
            }
            Pending::Resolve { doc } => {
                let Some(d) = docs.get_mut(doc) else { return };
                let changes = text_edits(&d.editor, &v["additionalTextEdits"], utf32);
                d.editor.apply_extra_edits(changes);
            }
            Pending::Hover { doc, at } => {
                if let Some(d) = docs.get_mut(doc) {
                    d.editor.deco.hover = markup(&v["contents"]).map(|t| (at, t));
                }
            }
            Pending::Definition => {
                let loc = match &v {
                    Value::Array(a) => a.first().cloned(),
                    Value::Object(_) => Some(v.clone()),
                    _ => None,
                };
                if let Some(loc) = loc {
                    let uri = loc["uri"].as_str().or_else(|| loc["targetUri"].as_str());
                    let range = if loc["targetSelectionRange"].is_object() {
                        &loc["targetSelectionRange"]
                    } else {
                        &loc["range"]
                    };
                    if let Some(path) = uri.and_then(uri::to_path) {
                        effects.push(Effect::Open {
                            path,
                            line: range["start"]["line"].as_u64().unwrap_or(0) as usize,
                            col: range["start"]["character"].as_u64().unwrap_or(0) as usize,
                        });
                    }
                }
            }
            Pending::Signature { doc } => {
                let Some(d) = docs.get_mut(doc) else { return };
                d.editor.deco.signature = signature(&v);
            }
            Pending::Semantic { doc, version } => {
                let Some(d) = docs.get_mut(doc) else { return };
                if d.editor.core.doc.version() != version {
                    return; // stale; a new request is due
                }
                if let Some(ra) = self.ra() {
                    d.editor.deco.semantic = semantic(&d.editor, &v["data"], ra);
                }
            }
        }
    }

    fn send_due_semantic(&mut self, docs: &Docs) {
        let now = Instant::now();
        let due: Vec<DocId> = self
            .semantic_due
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(id, _)| *id)
            .collect();
        if let Some(next) = self.semantic_due.values().filter(|t| **t > now).min() {
            self.ctx.request_repaint_after(*next - now);
        }
        for id in due {
            if self
                .pending
                .values()
                .any(|p| matches!(p, Pending::Semantic { doc, .. } if *doc == id))
            {
                continue;
            }
            self.semantic_due.remove(&id);
            let (Some(doc), Some(ra)) = (docs.get(id), self.ra()) else {
                continue;
            };
            let rid = ra.client.request(
                "textDocument/semanticTokens/full",
                json!({ "textDocument": { "uri": uri::from_path(&doc.path) } }),
            );
            self.pending.insert(
                rid,
                Pending::Semantic {
                    doc: id,
                    version: doc.editor.core.doc.version(),
                },
            );
        }
    }
}

fn text_edits(ed: &EditorState, edits: &Value, utf32: bool) -> Vec<Change> {
    edits
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let s = to_char(ed, &e["range"]["start"], utf32);
            let t = to_char(ed, &e["range"]["end"], utf32);
            Change::replace(s..t.max(s), e["newText"].as_str().unwrap_or_default())
        })
        .collect()
}

fn convert_item(ed: &EditorState, it: &Value, index: usize, at: usize, utf32: bool) -> Item {
    let label = it["label"].as_str().unwrap_or_default().to_owned();
    let (insert, start, end) = if it["textEdit"].is_object() {
        let te = &it["textEdit"];
        let range = if te["range"].is_object() {
            &te["range"]
        } else {
            &te["insert"]
        };
        (
            te["newText"].as_str().unwrap_or(&label).to_owned(),
            to_char(ed, &range["start"], utf32),
            to_char(ed, &range["end"], utf32),
        )
    } else {
        let (s, _) = ed.core.word_before_cursor();
        (
            it["insertText"].as_str().unwrap_or(&label).to_owned(),
            s.min(at),
            at,
        )
    };
    let deprecated =
        it["deprecated"] == true || it["tags"].as_array().is_some_and(|t| t.contains(&json!(1)));
    let additional = text_edits(ed, &it["additionalTextEdits"], utf32);
    Item {
        label_detail: it["labelDetails"]["detail"].as_str().map(str::to_owned),
        detail: it["labelDetails"]["description"]
            .as_str()
            .or_else(|| it["detail"].as_str())
            .map(str::to_owned),
        kind: kind(it["kind"].as_u64(), &label),
        filter_text: it["filterText"].as_str().unwrap_or(&label).to_owned(),
        sort_text: it["sortText"].as_str().unwrap_or(&label).to_owned(),
        is_snippet: it["insertTextFormat"].as_u64() == Some(2),
        insert_text: insert,
        prefix_len: at.saturating_sub(start),
        suffix_len: end.saturating_sub(at),
        resolved: it.get("data").is_none() || !additional.is_empty(),
        additional_edits: additional,
        documentation: markup(&it["documentation"]),
        deprecated,
        preselect: it["preselect"] == true,
        server_index: index,
        label,
    }
}

fn signature(v: &Value) -> Option<Signature> {
    let sigs = v["signatures"].as_array()?;
    let active_sig = v["activeSignature"].as_u64().unwrap_or(0) as usize;
    let sig = sigs.get(active_sig).or(sigs.first())?;
    let label = sig["label"].as_str()?.to_owned();
    let active_param = sig["activeParameter"]
        .as_u64()
        .or(v["activeParameter"].as_u64())
        .unwrap_or(0) as usize;
    let active = sig["parameters"]
        .as_array()
        .and_then(|ps| ps.get(active_param))
        .and_then(|p| {
            match &p["label"] {
                Value::Array(r) if r.len() == 2 => {
                    // Offsets are UTF-16 units into the label.
                    let (a, b) = (r[0].as_u64()? as usize, r[1].as_u64()? as usize);
                    Some(utf16_to_col(&label, a)..utf16_to_col(&label, b))
                }
                Value::String(s) => {
                    let byte = label.find(s.as_str())?;
                    let start = label[..byte].chars().count();
                    Some(start..start + s.chars().count())
                }
                _ => None,
            }
        });
    Some(Signature {
        label,
        active,
        documentation: markup(&sig["documentation"]),
    })
}

fn semantic(
    ed: &EditorState,
    data: &Value,
    ra: &RustAnalyzer,
) -> Vec<(std::ops::Range<usize>, Hl)> {
    let Some(data) = data.as_array() else {
        return Vec::new();
    };
    let nums: Vec<usize> = data
        .iter()
        .map(|n| n.as_u64().unwrap_or(0) as usize)
        .collect();
    let mods_of = |bits: usize| -> Vec<&str> {
        ra.token_modifiers
            .iter()
            .enumerate()
            .filter(|(i, _)| bits & (1 << i) != 0)
            .map(|(_, s)| s.as_str())
            .collect()
    };
    let mut out = Vec::new();
    let (mut line, mut col) = (0usize, 0usize);
    for t in nums.as_chunks::<5>().0 {
        if t[0] > 0 {
            line += t[0];
            col = t[1];
        } else {
            col += t[1];
        }
        let ty = ra.token_types.get(t[3]).map_or("", String::as_str);
        let mods = mods_of(t[4]);
        let has = |m: &str| mods.contains(&m);
        let hl = match ty {
            "function" | "method" if has("unsafe") => Some(Hl::Unsafe),
            "function" => Some(Hl::Function),
            "method" => Some(Hl::Method),
            "macro" => Some(Hl::Macro),
            "struct" | "enum" | "interface" | "trait" | "typeAlias" | "union" | "class"
            | "type" | "typeParameter" => Some(Hl::Type),
            "builtinType" => Some(Hl::BuiltinType),
            "enumMember" | "constParameter" => Some(Hl::Constant),
            "variable" | "parameter" if has("mutable") => Some(Hl::Mutable),
            "variable" if has("static") || has("constant") => Some(Hl::Constant),
            "parameter" => Some(Hl::Parameter),
            "property" => Some(Hl::Property),
            "namespace" => Some(Hl::Namespace),
            "lifetime" => Some(Hl::Lifetime),
            "selfKeyword" => Some(Hl::SelfKw),
            "keyword" if has("unsafe") => Some(Hl::Unsafe),
            "attribute" | "decorator" => Some(Hl::Attribute),
            _ => None,
        };
        if let Some(hl) = hl {
            let start = ed.core.doc.char_of(Pos { line, col });
            out.push((start..start + t[2], hl));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use perox_editor::syntax::Lang;

    #[test]
    fn converts_a_rust_analyzer_completion_item() {
        let ed = EditorState::new("fn f() {\n    let m = HashMa\n}\n", Lang::Rust);
        let at = ed.core.doc.char_of(Pos { line: 1, col: 18 });
        let item = json!({
            "label": "HashMap",
            "labelDetails": { "detail": " (use std::collections::HashMap)", "description": "HashMap<{unknown}, {unknown}>" },
            "kind": 22,
            "sortText": "ffffffef",
            "filterText": "HashMap",
            "textEdit": { "range": { "start": { "line": 1, "character": 12 }, "end": { "line": 1, "character": 18 } }, "newText": "HashMap" },
            "data": { "imports": [] }
        });
        let it = convert_item(&ed, &item, 3, at, true);
        assert_eq!(it.prefix_len, 6);
        assert_eq!(it.suffix_len, 0);
        assert_eq!(it.kind, Kind::Struct);
        assert!(!it.resolved, "import items need a resolve on accept");
        assert_eq!(it.server_index, 3);
    }

    #[test]
    fn signature_active_parameter() {
        let v = json!({ "signatures": [{ "label": "fn push(&mut self, value: T)", "parameters": [{ "label": [8, 17] }, { "label": [19, 27] }] }], "activeParameter": 1 });
        let s = signature(&v).unwrap();
        assert_eq!(&s.label[s.active.clone().unwrap()], "value: T");
    }

    #[test]
    fn utf16_columns() {
        assert_eq!(utf16_to_col("a😀b", 3), 2);
        assert_eq!(utf16_to_col("abc", 9), 3);
    }
}
