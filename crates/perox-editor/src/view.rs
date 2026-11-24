//! The egui editor widget. It draws only the lines on screen, turns keyboard and mouse
//! input into [`EditorCore`] operations, and shows the completion popup, hover and
//! signature help. Language-server requests go out through [`Output::requests`]; the
//! app answers them by filling `EditorState::popup` and `EditorState::deco`.

use std::ops::Range;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Event, EventFilter, FontId, Key, Modifiers, Pos2, Rect,
    Sense, Stroke, TextFormat, Vec2, text::LayoutJob,
};

use crate::completion::{Popup, Recency};
use crate::core::{EditorCore, word_at};
use crate::find::Find;
use crate::selection::{Selection, Selections};
use crate::syntax::{Hl, Lang, Syntax, line_sections};
use crate::theme::Theme;
use perox_icons::Icon;

const FONT_SIZE: f32 = 13.5;
const TAB_WIDTH: usize = 4;
const HOVER_DELAY: Duration = Duration::from_millis(450);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Signature {
    pub label: String,
    /// The active parameter, as a char range within `label`.
    pub active: Option<Range<usize>>,
    pub documentation: Option<String>,
}

/// What the language server has told us about this document.
#[derive(Debug, Clone, Default)]
pub struct Decorations {
    pub diagnostics: Vec<Diagnostic>,
    /// Semantic highlighting (char ranges, sorted), overriding tree-sitter's colours.
    pub semantic: Vec<(Range<usize>, Hl)>,
    /// Hover text for the char position it was requested at.
    pub hover: Option<(usize, String)>,
    pub signature: Option<Signature>,
    /// Inlay hints (types, parameter names), sorted by position.
    pub inlays: Vec<Inlay>,
    /// Version-control changes by line, for the gutter.
    pub line_marks: Vec<(usize, LineMark)>,
    /// Where the debugger is stopped in this file (0-based line).
    pub exec_line: Option<usize>,
    /// Breakpoint lines the debugger couldn't place (drawn hollow).
    pub unverified_breakpoints: Vec<usize>,
}

/// How a line differs from the last commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineMark {
    Added,
    Modified,
    /// Lines were deleted just above this one.
    DeletedAbove,
}

/// Text drawn inline that isn't part of the document, like `: Vec<u8>` after a binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inlay {
    /// Char index the hint is drawn before.
    pub at: usize,
    /// What's drawn, padding included.
    pub label: String,
}

/// A request for the language server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// `trigger`: the character that triggered it (`.`, `:`), if any.
    Completion {
        at: usize,
        trigger: Option<char>,
    },
    Hover {
        at: usize,
    },
    Definition {
        at: usize,
    },
    SignatureHelp {
        at: usize,
    },
    /// Fetch an accepted item's extra edits (auto-import `use` lines); apply them with
    /// [`EditorState::apply_extra_edits`].
    ResolveCompletion {
        server_index: usize,
    },
    References {
        at: usize,
    },
    Rename {
        at: usize,
        new_name: String,
    },
    CodeActions {
        range: Range<usize>,
    },
    Format,
}

#[derive(Debug, Default)]
pub struct Output {
    pub requests: Vec<Request>,
    /// The text changed this frame.
    pub changed: bool,
    /// The editor has keyboard focus.
    pub has_focus: bool,
    /// A breakpoint was added or removed by clicking the gutter.
    pub breakpoints_changed: bool,
}

/// One open editor: the document, its syntax tree, and view state.
pub struct EditorState {
    pub core: EditorCore,
    pub syntax: Option<Syntax>,
    pub deco: Decorations,
    pub popup: Option<Popup>,
    /// Where the open popup's request was made (char index).
    pub popup_requested_at: usize,
    /// Scroll so the primary cursor is visible on the next frame.
    pub reveal: bool,
    /// Take keyboard focus on the next frame.
    pub focus_requested: bool,
    pub find: Find,
    hover: Option<(usize, Instant, bool)>,
    drag_anchor: Option<usize>,
    lang: Lang,
    /// Edits not yet sent to the language server.
    lsp_edits: Vec<crate::text::Edited>,
    /// Breakpoints, as the char offsets of their lines' starts, so they move with edits.
    breakpoints: Vec<usize>,
}

impl std::fmt::Debug for EditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorState")
            .field("lang", &self.lang)
            .finish_non_exhaustive()
    }
}

impl EditorState {
    pub fn new(text: &str, lang: Lang) -> Self {
        let core = EditorCore::new(text);
        let syntax = Syntax::new(lang, core.doc.rope());
        let mut core = core;
        if lang != Lang::Rust {
            core.line_comment = Some("#");
        }
        Self {
            core,
            syntax,
            deco: Decorations::default(),
            popup: None,
            popup_requested_at: 0,
            reveal: false,
            focus_requested: false,
            find: Find::default(),
            hover: None,
            drag_anchor: None,
            lang,
            lsp_edits: Vec::new(),
            breakpoints: Vec::new(),
        }
    }

    /// Breakpoint lines (0-based, sorted).
    pub fn breakpoints(&self) -> Vec<usize> {
        let mut lines: Vec<usize> = self
            .breakpoints
            .iter()
            .map(|&c| self.core.doc.pos_of(c.min(self.core.doc.len_chars())).line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    /// Replaces the breakpoints (0-based lines; ones past the end are dropped).
    pub fn set_breakpoints(&mut self, lines: &[usize]) {
        let n = self.core.doc.len_lines();
        self.breakpoints = lines
            .iter()
            .filter(|&&l| l < n)
            .map(|&l| self.core.doc.rope().line_to_char(l))
            .collect();
    }

    /// Adds or removes the breakpoint on a line (0-based). True if it's now set.
    pub fn toggle_breakpoint(&mut self, line: usize) -> bool {
        let mut lines = self.breakpoints();
        let on = match lines.iter().position(|&l| l == line) {
            Some(i) => {
                lines.remove(i);
                false
            }
            None => {
                lines.push(line);
                true
            }
        };
        self.set_breakpoints(&lines);
        on
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Moves the cursor to a line/column and scrolls there.
    pub fn goto(&mut self, line: usize, col: usize) {
        let at = self.core.doc.char_of(crate::text::Pos { line, col });
        self.core.set_cursor(at, false);
        self.reveal = true;
    }

    /// Sets a fresh completion list (from the server) for a request made at `at`.
    pub fn set_completions(&mut self, mut popup: Popup, at: usize, recency: &Recency) {
        let head = self.core.sels.primary().head;
        let (word_start, typed) = self.core.word_before_cursor();
        if head < at.saturating_sub(64) || word_start > at {
            return; // the cursor moved away since the request
        }
        popup.word_start = word_start;
        popup.filter(&typed, recency);
        self.popup_requested_at = at;
        self.popup = (!popup.is_empty()).then_some(popup);
    }

    /// Feeds new edits to the syntax tree and queues them for the language server.
    pub fn sync(&mut self) {
        let edits = self.core.take_edits();
        if edits.is_empty() {
            return;
        }
        if let Some(s) = &mut self.syntax {
            s.update(&edits, self.core.doc.rope());
        }
        // Move decorations through the edits so nothing flickers or drifts while the
        // server works on fresh ones. Anything inside replaced text is dropped.
        use crate::text::map_through;
        let d = &mut self.deco;
        d.semantic.retain_mut(|(r, _)| {
            match (map_through(&edits, r.start), map_through(&edits, r.end)) {
                (Some(a), Some(b)) if a < b => {
                    *r = a..b;
                    true
                }
                _ => false,
            }
        });
        d.inlays.retain_mut(|i| match map_through(&edits, i.at) {
            Some(a) => {
                i.at = a;
                true
            }
            None => false,
        });
        self.breakpoints
            .retain_mut(|c| match map_through(&edits, *c) {
                Some(a) => {
                    *c = a;
                    true
                }
                None => false,
            });
        for diag in &mut d.diagnostics {
            let a = map_through(&edits, diag.range.start).unwrap_or(diag.range.start);
            let b = map_through(&edits, diag.range.end).unwrap_or(a);
            diag.range = a..b.max(a);
        }
        d.hover = None;
        self.lsp_edits.extend(edits);
    }

    /// Edits to send to the language server (`textDocument/didChange`), in order.
    pub fn take_lsp_edits(&mut self) -> Vec<crate::text::Edited> {
        self.sync();
        std::mem::take(&mut self.lsp_edits)
    }

    /// Types text at the cursors exactly as the keyboard would (auto-pairs, smart
    /// newline, completion triggers). Returns the language-server requests it causes.
    pub fn type_text(&mut self, text: &str, recency: &Recency) -> Vec<Request> {
        let mut out = Output::default();
        for chunk in split_typing(text) {
            if chunk == "\n" {
                self.core.newline();
                self.popup = None;
                self.sync();
            } else {
                type_and_trigger(self, &chunk, recency, &mut out);
            }
        }
        out.requests
    }

    /// Accepts the shown completion at `index` (0 = first). Returns follow-up requests.
    pub fn accept_completion_at(
        &mut self,
        index: usize,
        recency: &mut Recency,
    ) -> Option<Vec<Request>> {
        let popup = self.popup.as_mut()?;
        if index >= popup.shown.len() {
            return None;
        }
        popup.selected = index;
        let mut out = Output::default();
        accept_completion(self, recency, &mut out);
        self.sync();
        Some(out.requests)
    }

    /// Applies edits that arrive after a completion was accepted (auto-imports).
    pub fn apply_extra_edits(&mut self, changes: Vec<crate::text::Change>) {
        if changes.is_empty() {
            return;
        }
        let mut sorted = changes;
        sorted.sort_by_key(|c| c.range.start);
        let after = self.core.sels.map(&sorted);
        self.core.apply(sorted, after, crate::text::EditKind::Other);
        self.sync();
    }

    /// Applies edits given as byte ranges into the current text (the form designer's
    /// edits), as one undo step.
    pub fn apply_byte_edits(&mut self, edits: &[(std::ops::Range<usize>, String)]) {
        if edits.is_empty() {
            return;
        }
        let rope = self.core.doc.rope();
        let len = rope.len_bytes();
        let changes: Vec<crate::text::Change> = edits
            .iter()
            .map(|(r, text)| {
                let a = rope.byte_to_char(r.start.min(len));
                let b = rope.byte_to_char(r.end.min(len));
                crate::text::Change::replace(a..b.max(a), text.clone())
            })
            .collect();
        let mut sorted = changes;
        sorted.sort_by_key(|c| c.range.start);
        let after = self.core.sels.map(&sorted);
        self.core.apply(sorted, after, crate::text::EditKind::Other);
        self.core.doc.seal_undo();
        self.sync();
    }

    pub fn undo(&mut self) {
        self.core.undo();
        self.sync();
    }

    pub fn redo(&mut self) {
        self.core.redo();
        self.sync();
    }

    /// Replaces the text (e.g. reloaded from disk), as one undoable edit.
    pub fn set_text(&mut self, text: &str) {
        let sels = self.core.sels.clone();
        let edits = self.core.doc.replace_all(text, &sels);
        self.core.edits.extend(edits);
        self.core.sels = self.core.sels.clamp(self.core.doc.len_chars());
        self.sync();
    }
}

struct Metrics {
    font: FontId,
    line_h: f32,
    char_w: f32,
    gutter_w: f32,
}

/// Inlays on one line: (char column, width in cells).
type LineInlays = Vec<(usize, usize)>;

fn inlays_on(st: &EditorState, line: usize) -> LineInlays {
    if st.deco.inlays.is_empty() {
        return Vec::new();
    }
    let rope = st.core.doc.rope();
    let start = rope.line_to_char(line);
    let end = start + st.core.doc.line_len(line);
    st.deco
        .inlays
        .iter()
        .filter(|i| i.at >= start && i.at <= end)
        .map(|i| (i.at - start, i.label.chars().count()))
        .collect()
}

/// Display cell of `col`: tabs widened, inlays before it added. With `after_inlay`, an
/// inlay exactly at `col` counts too (where the char itself is drawn); without, it doesn't
/// (where the cursor sits).
fn cell(line: &str, inlays: &[(usize, usize)], col: usize, after_inlay: bool) -> usize {
    display_col(line, col)
        + inlays
            .iter()
            .filter(|(c, _)| *c < col || (after_inlay && *c == col))
            .map(|(_, w)| w)
            .sum::<usize>()
}

/// The char column under display cell `dcol`; a click on an inlay lands where it's drawn.
fn col_at_cell(line: &str, inlays: &[(usize, usize)], dcol: usize) -> usize {
    let n = line.chars().count();
    for col in 0..=n {
        let left = cell(line, inlays, col, true);
        if col == n {
            return n;
        }
        let w = if line.chars().nth(col) == Some('\t') {
            TAB_WIDTH
        } else {
            1
        };
        if dcol <= left + w / 2 && dcol < left + w || dcol < left {
            return col;
        }
    }
    n
}

fn display_col(line: &str, col: usize) -> usize {
    line.chars()
        .take(col)
        .map(|c| if c == '\t' { TAB_WIDTH } else { 1 })
        .sum()
}

/// Shows the editor. Call once per frame for the visible editor.
pub fn show(
    ui: &mut egui::Ui,
    st: &mut EditorState,
    recency: &mut Recency,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
) -> Output {
    let mut out = Output::default();
    let theme = Theme::for_visuals(ui.visuals());
    let font = FontId::monospace(FONT_SIZE);
    let (line_h, char_w) = ui
        .ctx()
        .fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'M')));
    let n_lines = st.core.doc.len_lines();
    let digits = n_lines.max(1).to_string().len().max(3);
    let m = Metrics {
        font,
        line_h,
        char_w,
        gutter_w: digits as f32 * char_w + 38.0,
    };
    let id = ui.make_persistent_id(id_salt);

    if std::mem::take(&mut st.focus_requested) {
        ui.memory_mut(|mem| mem.request_focus(id));
        st.reveal = true;
    }
    if st.find.open && find_bar(ui, st) {
        ui.memory_mut(|mem| mem.request_focus(id));
    }

    // ── keyboard (before layout, so this frame shows the result) ────────────
    let has_focus = ui.memory(|mem| mem.has_focus(id));
    if has_focus {
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        handle_keys(ui, st, recency, &mut out);
    }

    let max_cols = longest_line(st) + 8;
    let content = Vec2::new(
        m.gutter_w + max_cols as f32 * m.char_w,
        (n_lines as f32 + 3.0) * m.line_h,
    );
    let reveal = std::mem::take(&mut st.reveal);

    let mut scroll = egui::ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink(false);
    if reveal {
        let head = st.core.sels.primary().head;
        let pos = st.core.doc.pos_of(head);
        let y = pos.line as f32 * m.line_h;
        let x = cell(
            &st.core.doc.line_text(pos.line),
            &inlays_on(st, pos.line),
            pos.col,
            false,
        ) as f32
            * m.char_w;
        let mem_offset =
            egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(id.with("scroll")))
                .map(|s| s.clamped_offset())
                .unwrap_or_default();
        let avail = ui.available_size();
        let mut off = mem_offset;
        if y < off.y {
            off.y = (y - m.line_h * 3.0).max(0.0);
        } else if y + m.line_h > off.y + avail.y {
            off.y = y - avail.y + m.line_h * 4.0;
        }
        let text_w = avail.x - m.gutter_w;
        if x < off.x {
            off.x = (x - m.char_w * 8.0).max(0.0);
        } else if x > off.x + text_w - m.char_w * 2.0 {
            off.x = x - text_w + m.char_w * 8.0;
        }
        scroll = scroll.scroll_offset(off);
    }

    scroll.show_viewport(ui, |ui, viewport| {
        ui.set_min_size(content);
        let origin = ui.min_rect().min;
        let rect = Rect::from_min_size(
            origin,
            content.max(ui.available_size() + viewport.min.to_vec2()),
        );
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
        let text_x = origin.x + m.gutter_w;
        let to_char = |st: &EditorState, p: Pos2| -> usize {
            let line = (((p.y - origin.y) / m.line_h).floor().max(0.0) as usize)
                .min(n_lines.saturating_sub(1));
            let text = st.core.doc.line_text(line);
            let dcol = ((p.x - text_x) / m.char_w).round().max(0.0) as usize;
            st.core.doc.char_of(crate::text::Pos {
                line,
                col: col_at_cell(&text, &inlays_on(st, line), dcol),
            })
        };

        // The gutter's left part toggles breakpoints (on top of the text's click area).
        let gutter = Rect::from_min_size(
            Pos2::new(origin.x + viewport.min.x, origin.y),
            Vec2::new((m.gutter_w - 8.0).max(0.0), rect.height()),
        );
        let g = ui.interact(gutter, id.with("gutter"), Sense::click());
        let hover_line = g
            .hover_pos()
            .map(|p| ((p.y - origin.y) / m.line_h).floor().max(0.0) as usize);
        if g.clicked()
            && let Some(l) = hover_line.filter(|&l| l < n_lines)
        {
            st.toggle_breakpoint(l);
            out.breakpoints_changed = true;
        }
        let hover_line = hover_line.filter(|&l| l < n_lines);
        if g.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        handle_mouse(ui, st, &resp, &to_char, &mut out);

        let first = ((viewport.min.y / m.line_h).floor().max(0.0)) as usize;
        let last = (((viewport.max.y / m.line_h).ceil()) as usize + 1).min(n_lines);
        let painter = ui.painter_at(Rect::from_min_size(
            origin + viewport.min.to_vec2(),
            viewport.size(),
        ));
        paint(
            ui,
            &painter,
            st,
            &m,
            &theme,
            origin,
            viewport,
            first..last,
            has_focus,
            hover_line,
        );

        // Popups anchor to the primary cursor.
        let head = st.core.sels.primary().head;
        let hp = st.core.doc.pos_of(head);
        let cursor_pos = Pos2::new(
            text_x
                + cell(
                    &st.core.doc.line_text(hp.line),
                    &inlays_on(st, hp.line),
                    hp.col,
                    false,
                ) as f32
                    * m.char_w,
            origin.y + hp.line as f32 * m.line_h,
        );
        if has_focus {
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: Default::default(),
                    rect: resp.rect,
                    cursor_rect: Rect::from_min_size(cursor_pos, Vec2::new(2.0, m.line_h)),
                    should_interrupt_composition: false,
                });
            });
        }
        if let Some(sig) = &st.deco.signature
            && has_focus
        {
            signature_ui(ui, sig, cursor_pos, &m);
        }
        if st.popup.is_some()
            && has_focus
            && let Some(accept) = popup_ui(
                ui,
                st,
                cursor_pos + Vec2::new(-m.char_w * 2.0, m.line_h + 2.0),
                &m,
            )
        {
            st.popup.as_mut().expect("checked").selected = accept;
            accept_completion(st, recency, &mut out);
            ui.memory_mut(|mem| mem.request_focus(id));
        }
        hover_ui(ui, st, &resp, &to_char, &mut out);
    });

    out.changed |= !st.core.edits.is_empty() || !st.lsp_edits.is_empty();
    out.has_focus = ui.memory(|mem| mem.has_focus(id));
    st.sync();
    out
}

/// Splits typed text into keystrokes: one char at a time, newlines on their own.
fn split_typing(text: &str) -> Vec<String> {
    text.chars()
        .map(|c| {
            if c == '\n' {
                "\n".to_owned()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn longest_line(st: &EditorState) -> usize {
    // Cheap enough for files of tens of thousands of lines; revisit with a cache.
    let rope = st.core.doc.rope();
    rope.lines().map(|l| l.len_chars()).max().unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    painter: &egui::Painter,
    st: &EditorState,
    m: &Metrics,
    theme: &Theme,
    origin: Pos2,
    viewport: Rect,
    lines: Range<usize>,
    has_focus: bool,
    gutter_hover: Option<usize>,
) {
    let doc = &st.core.doc;
    let rope = doc.rope();
    let text_x = origin.x + m.gutter_w;
    let line_y = |l: usize| origin.y + l as f32 * m.line_h;
    let inlay_map: std::collections::HashMap<usize, LineInlays> = lines
        .clone()
        .map(|l| (l, inlays_on(st, l)))
        .filter(|(_, v)| !v.is_empty())
        .collect();
    let none: LineInlays = Vec::new();
    let inl = |l: usize| inlay_map.get(&l).unwrap_or(&none);
    // x of a char (drawn after any inlay at its column) and of a cursor (before it).
    let char_x =
        |l: usize, text: &str, col: usize| text_x + cell(text, inl(l), col, true) as f32 * m.char_w;
    let cursor_x = |l: usize, text: &str, col: usize| {
        text_x + cell(text, inl(l), col, false) as f32 * m.char_w
    };
    let primary_line = doc.pos_of(st.core.sels.primary().head).line;

    // Current line.
    if lines.contains(&primary_line) {
        let y = line_y(primary_line);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(origin.x + viewport.min.x, y),
                Pos2::new(origin.x + viewport.max.x, y + m.line_h),
            ),
            0.0,
            theme.current_line,
        );
    }

    // Where the debugger stopped.
    if let Some(l) = st.deco.exec_line.filter(|l| lines.contains(l)) {
        let y = line_y(l);
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(origin.x + viewport.min.x, y),
                Pos2::new(origin.x + viewport.max.x, y + m.line_h),
            ),
            0.0,
            theme.exec_line,
        );
    }

    // Selections.
    for sel in st.core.sels.iter().filter(|s| !s.is_empty()) {
        let r = sel.range();
        let (a, b) = (doc.pos_of(r.start), doc.pos_of(r.end));
        for l in a.line.max(lines.start)..=b.line.min(lines.end.saturating_sub(1)) {
            let text = doc.line_text(l);
            let c0 = if l == a.line { a.col } else { 0 };
            let c1 = if l == b.line { b.col } else { doc.line_len(l) };
            let extra = if l < b.line { m.char_w * 0.6 } else { 0.0 };
            let y = line_y(l);
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(char_x(l, &text, c0), y),
                    Pos2::new(cursor_x(l, &text, c1) + extra, y + m.line_h),
                ),
                2.0,
                theme.selection,
            );
        }
    }

    // Find matches.
    if st.find.open {
        for (i, r) in st.find.matches.iter().enumerate() {
            let (a, b) = (doc.pos_of(r.start), doc.pos_of(r.end));
            if a.line >= lines.end || b.line < lines.start {
                continue;
            }
            let current = st.find.current == Some(i);
            for l in a.line.max(lines.start)..=b.line.min(lines.end.saturating_sub(1)) {
                let text = doc.line_text(l);
                let c0 = if l == a.line { a.col } else { 0 };
                let c1 = if l == b.line { b.col } else { doc.line_len(l) };
                let y = line_y(l);
                let rect = Rect::from_min_max(
                    Pos2::new(char_x(l, &text, c0), y),
                    Pos2::new(cursor_x(l, &text, c1), y + m.line_h),
                );
                painter.rect_filled(
                    rect,
                    2.0,
                    if current {
                        theme.find_current
                    } else {
                        theme.find_match
                    },
                );
            }
        }
    }

    // Matching bracket.
    if let Some((a, b)) = matching_bracket(st) {
        for p in [a, b] {
            let pos = doc.pos_of(p);
            if lines.contains(&pos.line) {
                let text = doc.line_text(pos.line);
                let x = char_x(pos.line, &text, pos.col);
                let y = line_y(pos.line);
                painter.rect_stroke(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(m.char_w, m.line_h)),
                    2.0,
                    Stroke::new(1.0, theme.bracket),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    // Text.
    if !lines.is_empty() {
        let byte_range = rope.line_to_byte(lines.start)..if lines.end >= rope.len_lines() {
            rope.len_bytes()
        } else {
            rope.line_to_byte(lines.end)
        };
        let spans = st
            .syntax
            .as_ref()
            .map(|s| s.highlights(rope, byte_range))
            .unwrap_or_default();
        for l in lines.clone() {
            let text = doc.line_text(l);
            let line_char = rope.line_to_char(l);
            let mut sections = line_sections(&spans, rope.line_to_byte(l), &text);
            overlay_semantic(
                &mut sections,
                &st.deco.semantic,
                line_char,
                text.chars().count(),
            );
            let line_inlays: Vec<(usize, String)> = if st.deco.inlays.is_empty() {
                Vec::new()
            } else {
                let end = line_char + text.chars().count();
                st.deco
                    .inlays
                    .iter()
                    .filter(|i| i.at >= line_char && i.at <= end)
                    .map(|i| (i.at - line_char, i.label.clone()))
                    .collect()
            };
            let job = layout_line(&text, &sections, &line_inlays, &m.font, theme);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(Pos2::new(text_x, line_y(l)), galley, theme.text);
        }
    }

    // Diagnostics: squiggles, worst last so it's on top.
    let mut diags: Vec<&Diagnostic> = st.deco.diagnostics.iter().collect();
    diags.sort_by_key(|d| std::cmp::Reverse(d.severity));
    for d in diags {
        let (a, b) = (
            doc.pos_of(d.range.start),
            doc.pos_of(d.range.end.max(d.range.start)),
        );
        let color = theme.severity(d.severity);
        for l in a.line.max(lines.start)..=b.line.min(lines.end.saturating_sub(1)) {
            let text = doc.line_text(l);
            let c0 = if l == a.line { a.col } else { 0 };
            let mut c1 = if l == b.line { b.col } else { doc.line_len(l) };
            if c1 <= c0 {
                c1 = c0 + 1;
            }
            let y = line_y(l) + m.line_h - 2.0;
            squiggle(
                painter,
                char_x(l, &text, c0),
                cursor_x(l, &text, c1),
                y,
                color,
            );
        }
    }

    // Cursors.
    if has_focus {
        for sel in st.core.sels.iter() {
            let p = doc.pos_of(sel.head);
            if lines.contains(&p.line) {
                let text = doc.line_text(p.line);
                let x = cursor_x(p.line, &text, p.col);
                painter.rect_filled(
                    Rect::from_min_size(
                        Pos2::new(x - 1.0, line_y(p.line)),
                        Vec2::new(2.0, m.line_h),
                    ),
                    0.0,
                    theme.cursor,
                );
            }
        }
    }

    // Gutter last, over horizontally scrolled text.
    let gx = origin.x + viewport.min.x;
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(gx, origin.y + viewport.min.y),
            Pos2::new(gx + m.gutter_w - 8.0, origin.y + viewport.max.y),
        ),
        0.0,
        theme.gutter_bg,
    );
    let breakpoints = st.breakpoints();
    for l in lines {
        let worst = st
            .deco
            .diagnostics
            .iter()
            .filter(|d| doc.pos_of(d.range.start).line == l)
            .map(|d| d.severity)
            .min();
        let color = if l == primary_line {
            theme.text
        } else {
            theme.line_number
        };
        painter.text(
            Pos2::new(gx + m.gutter_w - 16.0, line_y(l)),
            Align2::RIGHT_TOP,
            (l + 1).to_string(),
            m.font.clone(),
            color,
        );
        if let Some((_, mark)) = st.deco.line_marks.iter().find(|(ml, _)| *ml == l) {
            let x = gx + m.gutter_w - 11.0;
            let y = line_y(l);
            match mark {
                LineMark::Added => painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(3.0, m.line_h)),
                    0.0,
                    theme.vcs_added,
                ),
                LineMark::Modified => painter.rect_filled(
                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(3.0, m.line_h)),
                    0.0,
                    theme.vcs_modified,
                ),
                LineMark::DeletedAbove => painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(x - 1.0, y - 3.0),
                        Pos2::new(x + 5.0, y),
                        Pos2::new(x - 1.0, y + 3.0),
                    ],
                    theme.vcs_deleted,
                    Stroke::NONE,
                )),
            };
        }
        let centre = Pos2::new(gx + 8.0, line_y(l) + m.line_h / 2.0);
        let radius = (m.line_h * 0.3).min(6.0);
        if breakpoints.contains(&l) {
            if st.deco.unverified_breakpoints.contains(&l) {
                painter.circle_stroke(centre, radius - 0.5, Stroke::new(1.5, theme.breakpoint));
            } else {
                painter.circle_filled(centre, radius, theme.breakpoint);
            }
        } else if gutter_hover == Some(l) {
            painter.circle_filled(centre, radius, theme.breakpoint.gamma_multiply(0.35));
        } else if let Some(sev) = worst {
            painter.circle_filled(
                Pos2::new(gx + 6.0, line_y(l) + m.line_h / 2.0),
                3.0,
                theme.severity(sev),
            );
        }
        if st.deco.exec_line == Some(l) {
            // An arrow pointing at the line, over the breakpoint if there is one.
            let (x, y, h) = (gx + 2.0, line_y(l) + m.line_h * 0.2, m.line_h * 0.6);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(x, y),
                    Pos2::new(x + 7.0, y),
                    Pos2::new(x + 13.0, y + h / 2.0),
                    Pos2::new(x + 7.0, y + h),
                    Pos2::new(x, y + h),
                ],
                theme.exec_arrow,
                // On a breakpoint, a red outline keeps the breakpoint visible.
                if breakpoints.contains(&l) {
                    Stroke::new(1.5, theme.breakpoint)
                } else {
                    Stroke::NONE
                },
            ));
        }
    }
}

fn overlay_semantic(
    sections: &mut Vec<(Range<usize>, Hl)>,
    semantic: &[(Range<usize>, Hl)],
    line_char: usize,
    len: usize,
) {
    let line_end = line_char + len;
    let relevant: Vec<(Range<usize>, Hl)> = semantic
        .iter()
        .filter(|(r, _)| r.start < line_end && r.end > line_char)
        .map(|(r, h)| {
            (
                r.start.max(line_char) - line_char..r.end.min(line_end) - line_char,
                *h,
            )
        })
        .collect();
    if relevant.is_empty() {
        return;
    }
    let mut per_col: Vec<Option<Hl>> = vec![None; len];
    for (r, h) in sections.iter() {
        for c in r.clone() {
            if c < len {
                per_col[c] = Some(*h);
            }
        }
    }
    for (r, h) in relevant {
        for c in r {
            per_col[c] = Some(h);
        }
    }
    sections.clear();
    for (c, h) in per_col.into_iter().enumerate() {
        if let Some(h) = h {
            match sections.last_mut() {
                Some((r, last)) if *last == h && r.end == c => r.end = c + 1,
                _ => sections.push((c..c + 1, h)),
            }
        }
    }
}

// `col` walks one past the last char too, to place inlays at the end of the line.
#[allow(clippy::needless_range_loop)]
fn layout_line(
    text: &str,
    sections: &[(Range<usize>, Hl)],
    inlays: &[(usize, String)],
    font: &FontId,
    theme: &Theme,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let chars: Vec<char> = text.chars().collect();
    let mut style: Vec<Option<Hl>> = vec![None; chars.len()];
    for (r, hl) in sections {
        let (a, b) = (r.start.min(chars.len()), r.end.min(chars.len()));
        style[a..b.max(a)].fill(Some(*hl));
    }
    let fmt = |hl: Option<Hl>| TextFormat {
        font_id: font.clone(),
        color: hl.map_or(theme.text, |h| theme.color(h)),
        italics: matches!(hl, Some(Hl::Comment | Hl::DocComment)),
        ..Default::default()
    };
    let inlay_fmt = TextFormat {
        font_id: font.clone(),
        color: theme.inlay,
        background: theme.inlay_bg,
        ..Default::default()
    };
    let mut run = String::new();
    let mut run_style: Option<Hl> = None;
    let flush = |job: &mut LayoutJob, run: &mut String, st: Option<Hl>| {
        if !run.is_empty() {
            job.append(&run.replace('\t', &" ".repeat(TAB_WIDTH)), 0.0, fmt(st));
            run.clear();
        }
    };
    for col in 0..=chars.len() {
        for (_, label) in inlays.iter().filter(|(c, _)| *c == col) {
            flush(&mut job, &mut run, run_style);
            job.append(label, 0.0, inlay_fmt.clone());
        }
        let Some(&ch) = chars.get(col) else { break };
        if style[col] != run_style {
            flush(&mut job, &mut run, run_style);
            run_style = style[col];
        }
        run.push(ch);
    }
    flush(&mut job, &mut run, run_style);
    job
}

fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        points.push(Pos2::new(x, if up { y - 1.5 } else { y + 1.0 }));
        x += 2.5;
        up = !up;
    }
    if points.len() >= 2 {
        painter.add(egui::Shape::line(points, Stroke::new(1.2, color)));
    }
}

fn matching_bracket(st: &EditorState) -> Option<(usize, usize)> {
    let doc = &st.core.doc;
    let head = st.core.sels.primary().head;
    let pairs = [('(', ')'), ('[', ']'), ('{', '}')];
    let try_at = |p: usize| -> Option<(usize, usize)> {
        let c = doc.char_at(p)?;
        for (o, cl) in pairs {
            if c == o {
                let mut depth = 0;
                for i in p..(p + 20_000).min(doc.len_chars()) {
                    match doc.char_at(i) {
                        Some(x) if x == o => depth += 1,
                        Some(x) if x == cl => {
                            depth -= 1;
                            if depth == 0 {
                                return Some((p, i));
                            }
                        }
                        _ => {}
                    }
                }
            } else if c == cl {
                let mut depth = 0;
                let lo = p.saturating_sub(20_000);
                for i in (lo..=p).rev() {
                    match doc.char_at(i) {
                        Some(x) if x == cl => depth += 1,
                        Some(x) if x == o => {
                            depth -= 1;
                            if depth == 0 {
                                return Some((i, p));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    };
    try_at(head).or_else(|| head.checked_sub(1).and_then(try_at))
}

// ── find bar ─────────────────────────────────────────────────────────────

fn open_find(st: &mut EditorState, replace: bool) {
    let sel = st.core.sels.primary();
    if !sel.is_empty() {
        let text = st.core.doc.slice(sel.range());
        if !text.contains('\n') {
            st.find.query = text;
        }
    }
    st.find.open = true;
    st.find.show_replace |= replace;
    st.find.focus_query = true;
}

/// The find/replace bar above the text. Returns true when focus should go back to the text.
fn find_bar(ui: &mut egui::Ui, st: &mut EditorState) -> bool {
    let mut back_to_text = false;
    let head = st.core.sels.primary().head;
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let f = &mut st.find;
                let toggle = if f.show_replace {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                };
                if ui
                    .add(egui::Button::image(toggle.image(ui)).frame(false))
                    .on_hover_text("Replace")
                    .clicked()
                {
                    f.show_replace = !f.show_replace;
                }
                let q = ui.add(
                    egui::TextEdit::singleline(&mut f.query)
                        .hint_text("Find")
                        .desired_width(220.0),
                );
                if std::mem::take(&mut f.focus_query) {
                    q.request_focus();
                }
                for (flag, icon, tip) in [
                    (&mut f.case_sensitive, Icon::CaseSensitive, "Match case"),
                    (&mut f.whole_word, Icon::WholeWord, "Whole word"),
                    (&mut f.regex, Icon::Regex, "Regular expression"),
                ] {
                    if ui
                        .add(egui::Button::image(icon.image(ui)).selected(*flag))
                        .on_hover_text(tip)
                        .clicked()
                    {
                        *flag = !*flag;
                    }
                }
                f.update(&st.core, head);
                let f = &mut st.find;
                match (&f.error, f.matches.len(), f.current) {
                    (Some(e), _, _) => {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                    (None, 0, _) if !f.query.is_empty() => {
                        ui.weak("No results");
                    }
                    (None, n, Some(c)) => {
                        ui.weak(format!("{} of {n}", c + 1));
                    }
                    _ => {}
                }
                let enter = q.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                let shift = ui.input(|i| i.modifiers.shift);
                let alt = ui.input(|i| i.modifiers.alt);
                if ui
                    .add(egui::Button::image(Icon::ArrowUp.image(ui)).frame(false))
                    .on_hover_text("Previous (⇧Enter)")
                    .clicked()
                    || (enter && shift)
                {
                    f.step(&mut st.core, true);
                    st.reveal = true;
                    q.request_focus();
                } else if ui
                    .add(egui::Button::image(Icon::ArrowDown.image(ui)).frame(false))
                    .on_hover_text("Next (Enter)")
                    .clicked()
                    || (enter && !alt)
                {
                    f.step(&mut st.core, false);
                    st.reveal = true;
                    q.request_focus();
                } else if enter && alt {
                    f.select_all(&mut st.core);
                    f.open = false;
                    back_to_text = true;
                }
                if ui
                    .add(egui::Button::image(Icon::X.image(ui)).frame(false))
                    .on_hover_text("Close (Esc)")
                    .clicked()
                    || (q.has_focus() || q.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape))
                {
                    st.find.open = false;
                    back_to_text = true;
                }
            });
            if st.find.open && st.find.show_replace {
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    let f = &mut st.find;
                    ui.add(
                        egui::TextEdit::singleline(&mut f.replacement)
                            .hint_text("Replace")
                            .desired_width(220.0),
                    );
                    if ui
                        .add(egui::Button::image_and_text(
                            Icon::Replace.image(ui),
                            "Replace",
                        ))
                        .clicked()
                    {
                        f.replace_one(&mut st.core);
                        st.reveal = true;
                    }
                    if ui
                        .add(egui::Button::image_and_text(
                            Icon::ReplaceAll.image(ui),
                            "Replace All",
                        ))
                        .clicked()
                    {
                        let n = f.replace_all(&mut st.core);
                        ui.ctx().request_repaint();
                        let _ = n;
                    }
                });
            }
        });
    if back_to_text {
        st.reveal = true;
    }
    back_to_text
}

// ── input ────────────────────────────────────────────────────────────────

fn word_mod(m: &Modifiers) -> bool {
    if cfg!(target_os = "macos") {
        m.alt
    } else {
        m.ctrl
    }
}

fn line_mod(m: &Modifiers) -> bool {
    cfg!(target_os = "macos") && m.mac_cmd
}

fn handle_keys(ui: &egui::Ui, st: &mut EditorState, recency: &mut Recency, out: &mut Output) {
    let events = ui.input(|i| i.events.clone());
    for event in events {
        match event {
            Event::Text(t) => {
                if t.chars().all(|c| c.is_control()) {
                    continue;
                }
                type_and_trigger(st, &t, recency, out);
            }
            Event::Ime(egui::ImeEvent::Commit(t)) => type_and_trigger(st, &t, recency, out),
            Event::Copy => ui.ctx().copy_text(st.core.copy()),
            Event::Cut => {
                let t = st.core.cut();
                ui.ctx().copy_text(t);
                close_popups(st);
            }
            Event::Paste(t) => {
                st.core.paste(&t);
                close_popups(st);
                st.reveal = true;
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                if popup_key(st, key, &modifiers, recency, out) {
                    continue;
                }
                key_command(st, key, &modifiers, out);
            }
            _ => {}
        }
    }
}

/// Keys the completion popup takes while open. Returns true if handled.
fn popup_key(
    st: &mut EditorState,
    key: Key,
    m: &Modifiers,
    recency: &mut Recency,
    out: &mut Output,
) -> bool {
    let Some(popup) = &mut st.popup else {
        return false;
    };
    if m.any() && !m.shift_only() {
        return false;
    }
    match key {
        Key::ArrowDown => popup.move_selection(1),
        Key::ArrowUp => popup.move_selection(-1),
        Key::PageDown => popup.move_selection(8),
        Key::PageUp => popup.move_selection(-8),
        Key::Enter | Key::Tab => accept_completion(st, recency, out),
        Key::Escape => st.popup = None,
        _ => return false,
    }
    true
}

fn accept_completion(st: &mut EditorState, recency: &mut Recency, out: &mut Output) {
    let Some(popup) = st.popup.take() else { return };
    let Some(item) = popup.current().cloned() else {
        return;
    };
    let head = st.core.sels.primary().head;
    // The item's edit started `prefix_len` chars before the request position.
    let start = st.popup_requested_at.saturating_sub(item.prefix_len);
    let prefix_len = head.saturating_sub(start);
    recency.record(&item.label);
    if !item.resolved {
        out.requests.push(Request::ResolveCompletion {
            server_index: item.server_index,
        });
    }
    st.core.insert_completion(
        prefix_len,
        item.suffix_len,
        &item.insert_text,
        item.is_snippet,
        item.additional_edits.clone(),
    );
    st.reveal = true;
    // Calls: show the signature right away.
    if item.insert_text.contains('(')
        && matches!(
            item.kind,
            crate::completion::Kind::Function | crate::completion::Kind::Method
        )
    {
        out.requests.push(Request::SignatureHelp {
            at: st.core.sels.primary().head,
        });
    }
}

fn close_popups(st: &mut EditorState) {
    st.popup = None;
    st.deco.signature = None;
}

fn type_and_trigger(st: &mut EditorState, text: &str, recency: &Recency, out: &mut Output) {
    st.core.type_text(text);
    st.sync();
    st.reveal = true;
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        close_popups(st);
        return;
    };
    let head = st.core.sels.primary().head;
    let in_comment = st
        .syntax
        .as_ref()
        .is_some_and(|s| s.is_in_comment_or_string(st.core.doc.rope().char_to_byte(head)));
    if c.is_alphanumeric() || c == '_' {
        if in_comment {
            st.popup = None;
            return;
        }
        let (_, typed) = st.core.word_before_cursor();
        match &mut st.popup {
            Some(p) => {
                p.filter(&typed, recency);
                if p.is_empty() {
                    st.popup = None;
                }
                if st.popup.as_ref().is_none_or(|p| p.incomplete) {
                    out.requests.push(Request::Completion {
                        at: head,
                        trigger: None,
                    });
                }
            }
            None => out.requests.push(Request::Completion {
                at: head,
                trigger: None,
            }),
        }
        return;
    }
    st.popup = None;
    let prev = head.checked_sub(2).and_then(|i| st.core.doc.char_at(i));
    match c {
        '.' if !in_comment => out.requests.push(Request::Completion {
            at: head,
            trigger: Some('.'),
        }),
        ':' if prev == Some(':') && !in_comment => out.requests.push(Request::Completion {
            at: head,
            trigger: Some(':'),
        }),
        '(' | ',' if !in_comment => out.requests.push(Request::SignatureHelp { at: head }),
        ')' | ';' => st.deco.signature = None,
        _ => {}
    }
}

fn key_command(st: &mut EditorState, key: Key, m: &Modifiers, out: &mut Output) {
    let core = &mut st.core;
    let shift = m.shift;
    let mut moved = true;
    let mut edited = false;
    match key {
        Key::ArrowLeft if line_mod(m) => core.line_start(shift),
        Key::ArrowRight if line_mod(m) => core.line_end(shift),
        Key::ArrowLeft => core.move_left(shift, word_mod(m)),
        Key::ArrowRight => core.move_right(shift, word_mod(m)),
        Key::ArrowUp if m.command && m.alt => core.add_cursor_vertical(false),
        Key::ArrowDown if m.command && m.alt => core.add_cursor_vertical(true),
        Key::ArrowUp if line_mod(m) => core.doc_start(shift),
        Key::ArrowDown if line_mod(m) => core.doc_end(shift),
        Key::ArrowUp => core.move_vertical(-1, shift),
        Key::ArrowDown => core.move_vertical(1, shift),
        Key::PageUp => core.move_vertical(-30, shift),
        Key::PageDown => core.move_vertical(30, shift),
        Key::Home if m.command => core.doc_start(shift),
        Key::End if m.command => core.doc_end(shift),
        Key::Home => core.line_start(shift),
        Key::End => core.line_end(shift),
        Key::Backspace if line_mod(m) => {
            core.line_start(true);
            core.backspace();
            edited = true;
        }
        Key::Backspace if word_mod(m) => {
            core.delete_word_back();
            edited = true;
        }
        Key::Backspace => {
            core.backspace();
            // Keep the popup while deleting within the word.
            let (start, typed) = core.word_before_cursor();
            if let Some(p) = &mut st.popup {
                if start < p.word_start || typed.is_empty() {
                    st.popup = None;
                } else {
                    p.filter(&typed, &Recency::default());
                }
            }
            st.reveal = true;
            return;
        }
        Key::Delete if word_mod(m) => {
            core.delete_word_forward();
            edited = true;
        }
        Key::Delete => {
            core.delete_forward();
            edited = true;
        }
        Key::Enter => {
            core.newline();
            edited = true;
        }
        Key::Tab if shift => {
            if !core.snippet_next(true) {
                core.outdent_lines();
            }
            edited = true;
        }
        Key::Tab => {
            if !core.snippet_next(false) {
                core.tab();
            }
            edited = true;
        }
        Key::Escape => {
            st.deco.signature = None;
            core.collapse();
        }
        Key::A if m.command => core.select_all(),
        Key::Z if m.command && shift => {
            core.redo();
            edited = true;
        }
        Key::Z if m.command => {
            core.undo();
            edited = true;
        }
        Key::Y if m.command && !cfg!(target_os = "macos") => {
            core.redo();
            edited = true;
        }
        Key::D if m.command => core.select_next_occurrence(),
        Key::Slash if m.command => {
            core.toggle_comment();
            edited = true;
        }
        Key::CloseBracket if m.command => {
            core.indent_lines();
            edited = true;
        }
        Key::OpenBracket if m.command => {
            core.outdent_lines();
            edited = true;
        }
        Key::F if m.command => {
            open_find(st, m.alt);
            return;
        }
        Key::H if m.ctrl && !cfg!(target_os = "macos") => {
            open_find(st, true);
            return;
        }
        Key::G if m.command && !m.ctrl => {
            st.find.step(&mut st.core, m.shift);
            st.reveal = true;
            return;
        }
        Key::F3 => {
            st.find.step(&mut st.core, m.shift);
            st.reveal = true;
            return;
        }
        Key::Space if m.ctrl => {
            out.requests.push(Request::Completion {
                at: core.sels.primary().head,
                trigger: None,
            });
            moved = false;
        }
        Key::F12 => {
            out.requests.push(Request::Definition {
                at: core.sels.primary().head,
            });
            moved = false;
        }
        _ => moved = false,
    }
    if moved || edited {
        st.popup = None;
        st.reveal = true;
    }
    if edited {
        st.deco.signature = None;
    }
}

fn handle_mouse(
    ui: &egui::Ui,
    st: &mut EditorState,
    resp: &egui::Response,
    to_char: &dyn Fn(&EditorState, Pos2) -> usize,
    out: &mut Output,
) {
    let modifiers = ui.input(|i| i.modifiers);
    if resp.clicked() || resp.drag_started() || resp.double_clicked() || resp.triple_clicked() {
        resp.request_focus();
    }
    let Some(p) = resp.interact_pointer_pos() else {
        if !ui.input(|i| i.pointer.primary_down()) {
            st.drag_anchor = None;
        }
        return;
    };
    let at = to_char(st, p);
    if resp.triple_clicked() {
        let line = st.core.doc.pos_of(at).line;
        let start = st.core.doc.rope().line_to_char(line);
        let end = if line + 1 < st.core.doc.len_lines() {
            st.core.doc.rope().line_to_char(line + 1)
        } else {
            st.core.doc.len_chars()
        };
        st.core.select_range(start..end);
    } else if resp.double_clicked() {
        let (s, e) = word_at(&st.core.doc, at);
        st.core.select_range(s..e.max(s));
    } else if resp.drag_started() {
        st.popup = None;
        if modifiers.command && !modifiers.alt {
            out.requests.push(Request::Definition { at });
            st.core.set_cursor(at, false);
        } else if modifiers.alt {
            st.core.add_cursor(at);
        } else {
            st.core.set_cursor(at, modifiers.shift);
            st.drag_anchor = Some(if modifiers.shift {
                st.core.sels.primary().anchor
            } else {
                at
            });
        }
    } else if resp.dragged() {
        if let Some(anchor) = st.drag_anchor {
            st.core.sels = Selections::single(Selection::new(anchor, at));
            st.reveal = true;
        }
    } else if resp.clicked() && st.drag_anchor.is_none() {
        st.popup = None;
        if modifiers.command && !modifiers.alt {
            out.requests.push(Request::Definition { at });
        } else if modifiers.alt {
            st.core.add_cursor(at);
        } else {
            st.core.set_cursor(at, modifiers.shift);
        }
    }
}

// ── popups ───────────────────────────────────────────────────────────────

fn popup_ui(ui: &egui::Ui, st: &EditorState, pos: Pos2, m: &Metrics) -> Option<usize> {
    let popup = st.popup.as_ref()?;
    let theme = Theme::for_visuals(ui.visuals());
    let mut clicked = None;
    let rows = popup.shown.len().min(12);
    let row_h = m.line_h + 4.0;
    let area = egui::Area::new(ui.scope_id().with("completion"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            ui.horizontal_top(|ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(4)
                    .show(ui, |ui| {
                        // Frames inherit the row layout; the list itself is a column.
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(460.0);
                            egui::ScrollArea::vertical()
                                .max_height(rows as f32 * row_h + 4.0)
                                .show(ui, |ui| {
                                    for (row, idx) in popup.shown.iter().enumerate() {
                                        let item = &popup.items[*idx];
                                        let selected = row == popup.selected;
                                        let (rect, resp) = ui.allocate_exact_size(
                                            Vec2::new(ui.available_width(), row_h),
                                            Sense::click(),
                                        );
                                        if selected {
                                            ui.painter().rect_filled(rect, 3.0, theme.selection);
                                            if ui.input(|i| {
                                                i.key_pressed(Key::ArrowDown)
                                                    || i.key_pressed(Key::ArrowUp)
                                            }) {
                                                resp.scroll_to_me(None);
                                            }
                                        }
                                        let y = rect.center().y;
                                        let mut x = rect.left() + 4.0;
                                        ui.painter().text(
                                            Pos2::new(x, y),
                                            Align2::LEFT_CENTER,
                                            item.kind.tag(),
                                            FontId::monospace(10.0),
                                            theme.kind_color(item.kind),
                                        );
                                        x += 24.0;
                                        let label_color = if item.deprecated {
                                            theme.line_number
                                        } else {
                                            theme.text
                                        };
                                        let r = ui.painter().text(
                                            Pos2::new(x, y),
                                            Align2::LEFT_CENTER,
                                            &item.label,
                                            m.font.clone(),
                                            label_color,
                                        );
                                        if let Some(d) = &item.label_detail {
                                            ui.painter().text(
                                                Pos2::new(r.right(), y),
                                                Align2::LEFT_CENTER,
                                                d,
                                                m.font.clone(),
                                                theme.line_number,
                                            );
                                        }
                                        if let Some(detail) = &item.detail {
                                            let max_chars = 28;
                                            let short: String =
                                                if detail.chars().count() > max_chars {
                                                    detail
                                                        .chars()
                                                        .take(max_chars - 1)
                                                        .chain(['…'])
                                                        .collect()
                                                } else {
                                                    detail.clone()
                                                };
                                            ui.painter().text(
                                                Pos2::new(rect.right() - 6.0, y),
                                                Align2::RIGHT_CENTER,
                                                short,
                                                FontId::monospace(11.0),
                                                theme.line_number,
                                            );
                                        }
                                        if resp.clicked() {
                                            clicked = Some(row);
                                        }
                                    }
                                });
                        });
                    });
                if let Some(item) = popup.current()
                    && (item.documentation.is_some() || item.detail.is_some())
                {
                    egui::Frame::popup(ui.style())
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(380.0);
                            egui::ScrollArea::vertical()
                                .max_height(300.0)
                                .id_salt("doc")
                                .show(ui, |ui| {
                                    ui.set_width(370.0);
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            if let Some(d) = &item.detail {
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(d).monospace(),
                                                    )
                                                    .wrap(),
                                                );
                                            }
                                            if let Some(doc) = &item.documentation {
                                                ui.separator();
                                                markdown(ui, doc);
                                            }
                                        },
                                    );
                                });
                        });
                }
            });
        });
    let _ = area;
    clicked
}

fn signature_ui(ui: &egui::Ui, sig: &Signature, cursor: Pos2, m: &Metrics) {
    let theme = Theme::for_visuals(ui.visuals());
    egui::Area::new(ui.scope_id().with("signature"))
        .order(egui::Order::Foreground)
        .pivot(Align2::LEFT_BOTTOM)
        .fixed_pos(cursor - Vec2::new(m.char_w * 2.0, 4.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(6)
                .show(ui, |ui| {
                    ui.set_max_width(640.0);
                    let mut job = LayoutJob::default();
                    let chars: Vec<char> = sig.label.chars().collect();
                    let (a, b) = sig.active.clone().map_or((0, 0), |r| {
                        (r.start.min(chars.len()), r.end.min(chars.len()))
                    });
                    let fmt = |color: Color32, underline: bool| TextFormat {
                        font_id: m.font.clone(),
                        color,
                        underline: if underline {
                            Stroke::new(1.0, color)
                        } else {
                            Stroke::NONE
                        },
                        ..Default::default()
                    };
                    job.append(
                        &chars[..a].iter().collect::<String>(),
                        0.0,
                        fmt(theme.text, false),
                    );
                    job.append(
                        &chars[a..b].iter().collect::<String>(),
                        0.0,
                        fmt(theme.color(Hl::Parameter), true),
                    );
                    job.append(
                        &chars[b..].iter().collect::<String>(),
                        0.0,
                        fmt(theme.text, false),
                    );
                    ui.label(job);
                    if let Some(doc) = &sig.documentation {
                        ui.separator();
                        markdown(ui, doc);
                    }
                });
        });
}

fn hover_ui(
    ui: &egui::Ui,
    st: &mut EditorState,
    resp: &egui::Response,
    to_char: &dyn Fn(&EditorState, Pos2) -> usize,
    out: &mut Output,
) {
    let Some(p) = resp.hover_pos() else {
        st.hover = None;
        return;
    };
    if ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let at = to_char(st, p);
    // Diagnostics under the pointer show immediately.
    let diags: Vec<&Diagnostic> = st
        .deco
        .diagnostics
        .iter()
        .filter(|d| d.range.start <= at && at <= d.range.end.max(d.range.start + 1))
        .collect();
    match st.hover {
        Some((pos, since, requested)) if pos == at => {
            if !requested && since.elapsed() >= HOVER_DELAY {
                out.requests.push(Request::Hover { at });
                st.hover = Some((pos, since, true));
            } else if !requested {
                ui.ctx().request_repaint_after(HOVER_DELAY);
            }
        }
        _ => {
            st.hover = Some((at, Instant::now(), false));
            ui.ctx().request_repaint_after(HOVER_DELAY);
        }
    }
    let hover_text = st.deco.hover.as_ref().filter(|(pos, _)| {
        let (s, e) = word_at(&st.core.doc, *pos);
        (s..=e.max(s)).contains(&at)
    });
    if diags.is_empty() && hover_text.is_none() {
        return;
    }
    let theme = Theme::for_visuals(ui.visuals());
    egui::Area::new(ui.scope_id().with("hover"))
        .order(egui::Order::Tooltip)
        .fixed_pos(p + Vec2::new(8.0, 14.0))
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_max_width(560.0);
                    for d in &diags {
                        ui.colored_label(
                            theme.severity(d.severity),
                            format!("{} ({})", d.message, d.source),
                        );
                    }
                    if let Some((_, text)) = hover_text {
                        if !diags.is_empty() {
                            ui.separator();
                        }
                        markdown(ui, text);
                    }
                });
        });
}

/// Enough Markdown for rust-analyzer's hovers and docs: code fences, headings, and
/// paragraphs (consecutive lines flow together).
pub fn markdown(ui: &mut egui::Ui, text: &str) {
    let mut in_code = false;
    let mut code = String::new();
    let mut para = String::new();
    let flush = |ui: &mut egui::Ui, para: &mut String| {
        if !para.trim().is_empty() {
            let clean = para.trim().replace("**", "").replace('`', "");
            ui.add(egui::Label::new(clean).wrap());
        }
        para.clear();
    };
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            flush(ui, &mut para);
            if in_code {
                egui::Frame::new()
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(4)
                    .corner_radius(4)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(code.trim_end()).monospace())
                                .wrap(),
                        );
                    });
                code.clear();
            }
            in_code = !in_code;
            continue;
        }
        if in_code {
            code.push_str(line);
            code.push('\n');
        } else if let Some(h) = line.trim_start().strip_prefix('#') {
            flush(ui, &mut para);
            ui.strong(h.trim_start_matches('#').trim());
        } else if line.trim() == "---" {
            flush(ui, &mut para);
            ui.separator();
        } else if line.trim().is_empty() {
            flush(ui, &mut para);
        } else if line.trim_start().starts_with("- ") || line.trim_start().starts_with("* ") {
            flush(ui, &mut para);
            para.push_str("• ");
            para.push_str(&line.trim_start()[2..]);
        } else {
            if !para.is_empty() {
                para.push(' ');
            }
            para.push_str(line.trim());
        }
    }
    flush(ui, &mut para);
    if in_code && !code.is_empty() {
        ui.add(egui::Label::new(egui::RichText::new(code.trim_end()).monospace()).wrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints_follow_edits() {
        let mut st = EditorState::new("a\nb\nc\nd\n", Lang::Rust);
        assert!(st.toggle_breakpoint(2));
        st.set_breakpoints(&[1, 2, 99]);
        assert_eq!(st.breakpoints(), [1, 2]);
        // Two lines inserted above: both move down.
        st.apply_byte_edits(&[(0..0, "x\ny\n".into())]);
        st.sync();
        assert_eq!(st.breakpoints(), [3, 4]);
        // Typing at the start of a breakpoint's line keeps it on that line.
        let at = st.core.doc.rope().line_to_byte(3);
        st.apply_byte_edits(&[(at..at, "  ".into())]);
        st.sync();
        assert_eq!(st.breakpoints(), [3, 4]);
        assert!(!st.toggle_breakpoint(3));
        assert_eq!(st.breakpoints(), [4]);
    }

    #[test]
    fn tabs_count_as_four_columns() {
        assert_eq!(display_col("\tab", 1), 4);
        assert_eq!(display_col("\tab", 3), 6);
        assert_eq!(col_at_cell("\tab", &[], 5), 2);
        assert_eq!(col_at_cell("abc", &[], 9), 3);
    }

    #[test]
    fn inlays_shift_chars_but_not_the_cursor_before_them() {
        // `let x = 1;` with `: i32` (5 cells) drawn before col 5 (after `x`).
        let line = "let x = 1;";
        let inl = [(5, 5)];
        assert_eq!(
            cell(line, &inl, 5, false),
            5,
            "cursor right after x sits before the hint"
        );
        assert_eq!(
            cell(line, &inl, 5, true),
            10,
            "the space after x is drawn after the hint"
        );
        assert_eq!(cell(line, &inl, 6, false), 11);
        // Clicking on the hint puts the cursor where it's drawn; after it, on the next chars.
        assert_eq!(col_at_cell(line, &inl, 7), 5);
        assert_eq!(col_at_cell(line, &inl, 11), 6);
        assert_eq!(col_at_cell(line, &inl, 2), 2);
    }

    #[test]
    fn brackets_match() {
        let mut st = EditorState::new("f(a[1], {b})", Lang::Plain);
        st.core.set_cursor(1, false);
        assert_eq!(matching_bracket(&st), Some((1, 11)));
        st.core.set_cursor(12, false);
        assert_eq!(matching_bracket(&st), Some((1, 11)));
    }

    #[test]
    fn typing_dot_requests_completion() {
        let mut st = EditorState::new("fn f() { x }", Lang::Rust);
        st.core.set_cursor(10, false);
        let mut out = Output::default();
        type_and_trigger(&mut st, ".", &Recency::default(), &mut out);
        assert_eq!(
            out.requests,
            [Request::Completion {
                at: 11,
                trigger: Some('.')
            }]
        );
    }

    #[test]
    fn no_completion_inside_comments() {
        let mut st = EditorState::new("// hello \n", Lang::Rust);
        st.core.set_cursor(8, false);
        let mut out = Output::default();
        type_and_trigger(&mut st, "w", &Recency::default(), &mut out);
        assert!(out.requests.is_empty());
    }
}
