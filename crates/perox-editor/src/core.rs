//! Editing operations: everything a keystroke can do to a document and its cursors.
//! No UI here, so all of it is unit-tested.

use std::ops::Range;

use crate::selection::{Selection, Selections};
use crate::snippet::{Snippet, SnippetSession};
use crate::text::{Change, Document, EditKind, Edited};

pub const INDENT: &str = "    ";
const INDENT_WIDTH: usize = 4;

/// A document with cursors. Edits are collected in `edits` for syntax and LSP sync.
#[derive(Debug, Clone)]
pub struct EditorCore {
    pub doc: Document,
    pub sels: Selections,
    /// Applied since last drained; consumers: syntax highlighting, LSP sync.
    pub edits: Vec<Edited>,
    pub snippet: Option<SnippetSession>,
    /// Line comment token for toggle-comment; `None` disables it.
    pub line_comment: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Punct,
    Newline,
}

fn class(c: char) -> Class {
    if c == '\n' {
        Class::Newline
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Punct
    }
}

fn closer_for(c: char) -> Option<char> {
    match c {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        _ => None,
    }
}

impl EditorCore {
    pub fn new(text: &str) -> Self {
        Self {
            doc: Document::new(text),
            sels: Selections::single(Selection::cursor(0)),
            edits: Vec::new(),
            snippet: None,
            line_comment: Some("//"),
        }
    }

    pub fn text(&self) -> String {
        self.doc.text()
    }

    pub fn take_edits(&mut self) -> Vec<Edited> {
        std::mem::take(&mut self.edits)
    }

    /// Applies changes and sets selections; keeps the snippet session in sync.
    pub fn apply(&mut self, changes: Vec<Change>, after: Selections, kind: EditKind) {
        let mut sorted = changes;
        sorted.sort_by_key(|c| c.range.start);
        if let Some(snippet) = &mut self.snippet {
            snippet.map(&sorted);
        }
        let before = self.sels.clone();
        let edits = self.doc.apply(sorted, &before, after.clone(), kind);
        self.sels = after.clamp(self.doc.len_chars());
        self.edits.extend(edits);
        self.end_snippet_if_outside();
    }

    /// For each selection, `f` returns a change and where the cursor goes, as an offset
    /// into the change's new text (and optionally an anchor offset). Changes must stay
    /// within or next to their selection.
    fn edit_each(
        &mut self,
        kind: EditKind,
        mut f: impl FnMut(&Document, Selection) -> Option<(Change, usize, Option<usize>)>,
    ) {
        let mut changes = Vec::new();
        let mut new_sels = Vec::new();
        let mut delta: isize = 0;
        let mut last_end = 0usize;
        for sel in self.sels.iter().copied() {
            let Some((change, cursor_off, anchor_off)) = f(&self.doc, sel) else {
                let h = (sel.head as isize + delta) as usize;
                let a = (sel.anchor as isize + delta) as usize;
                new_sels.push(Selection::new(a, h));
                continue;
            };
            if change.range.start < last_end {
                continue; // would overlap the previous cursor's change
            }
            last_end = change.range.end;
            let start_new = (change.range.start as isize + delta) as usize;
            let head = start_new + cursor_off;
            let anchor = anchor_off.map_or(head, |a| start_new + a);
            new_sels.push(Selection::new(anchor, head));
            delta += change.text.chars().count() as isize - change.range.len() as isize;
            changes.push(change);
        }
        let primary = self
            .sels
            .primary_index()
            .min(new_sels.len().saturating_sub(1));
        let after = Selections::new(new_sels, primary);
        self.apply(changes, after, kind);
    }

    // ── typing ────────────────────────────────────────────────────────────

    /// Types text at every cursor (replacing selections), with bracket auto-pairing
    /// and over-typing of closers for single characters.
    pub fn type_text(&mut self, text: &str) {
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };
        if text == "\n" || text == "\r\n" {
            return self.newline();
        }
        self.edit_each(EditKind::Typing, |doc, sel| {
            let r = sel.range();
            if let Some(c) = single {
                let next = doc.char_at(r.end);
                let prev = r.start.checked_sub(1).and_then(|i| doc.char_at(i));
                // Over-type a closer that's already there.
                if sel.is_empty() && matches!(c, ')' | ']' | '}' | '"') && next == Some(c) {
                    return Some((
                        Change::replace(r.start..r.start + 1, c.to_string()),
                        1,
                        None,
                    ));
                }
                if let Some(close) = closer_for(c) {
                    if !sel.is_empty() {
                        // Wrap the selection.
                        let inner = doc.slice(r.clone());
                        let len = inner.chars().count();
                        return Some((
                            Change::replace(r, format!("{c}{inner}{close}")),
                            1 + len,
                            Some(1),
                        ));
                    }
                    let next_ok = next.is_none_or(|n| {
                        n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';')
                    });
                    let quote_ok =
                        c != '"' || prev.is_none_or(|p| !p.is_alphanumeric() && p != '"');
                    if next_ok && quote_ok {
                        return Some((Change::replace(r, format!("{c}{close}")), 1, None));
                    }
                }
            }
            let len = text.chars().count();
            Some((Change::replace(r, text), len, None))
        });
    }

    /// Enter: keeps the indentation; after an opening bracket, indents one more level
    /// and, if the closer follows, puts it on its own line.
    pub fn newline(&mut self) {
        self.edit_each(EditKind::Other, |doc, sel| {
            let r = sel.range();
            let line = doc.pos_of(r.start).line;
            let line_text = doc.line_text(line);
            let indent: String = line_text
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            let prev = r.start.checked_sub(1).and_then(|i| doc.char_at(i));
            let next = doc.char_at(r.end);
            let opens = matches!(prev, Some('{' | '(' | '['));
            let closes = matches!(
                (prev, next),
                (Some('{'), Some('}')) | (Some('('), Some(')')) | (Some('['), Some(']'))
            );
            let text = if closes {
                format!("\n{indent}{INDENT}\n{indent}")
            } else if opens {
                format!("\n{indent}{INDENT}")
            } else {
                format!("\n{indent}")
            };
            let cursor = 1 + indent.chars().count() + if opens { INDENT_WIDTH } else { 0 };
            Some((Change::replace(r, text), cursor, None))
        });
    }

    pub fn backspace(&mut self) {
        self.edit_each(EditKind::Deleting, |doc, sel| {
            let r = sel.range();
            if !sel.is_empty() {
                return Some((Change::delete(r), 0, None));
            }
            if r.start == 0 {
                return None;
            }
            let prev = doc.char_at(r.start - 1);
            let next = doc.char_at(r.start);
            // Delete an empty pair: (|)
            if let (Some(p), Some(n)) = (prev, next)
                && closer_for(p) == Some(n)
            {
                return Some((Change::delete(r.start - 1..r.start + 1), 0, None));
            }
            // In leading whitespace, delete back to the previous indent stop.
            let pos = doc.pos_of(r.start);
            let line = doc.line_text(pos.line);
            let before: String = line.chars().take(pos.col).collect();
            if !before.is_empty() && before.chars().all(|c| c == ' ') {
                let n = match pos.col % INDENT_WIDTH {
                    0 => INDENT_WIDTH,
                    m => m,
                };
                return Some((Change::delete(r.start - n..r.start), 0, None));
            }
            Some((Change::delete(r.start - 1..r.start), 0, None))
        });
    }

    pub fn delete_forward(&mut self) {
        self.edit_each(EditKind::Deleting, |doc, sel| {
            let r = sel.range();
            if !sel.is_empty() {
                return Some((Change::delete(r), 0, None));
            }
            (r.start < doc.len_chars()).then(|| (Change::delete(r.start..r.start + 1), 0, None))
        });
    }

    pub fn delete_word_back(&mut self) {
        self.edit_each(EditKind::Deleting, |doc, sel| {
            let r = sel.range();
            if !sel.is_empty() {
                return Some((Change::delete(r), 0, None));
            }
            let start = word_left(doc, r.start);
            (start < r.start).then(|| (Change::delete(start..r.start), 0, None))
        });
    }

    pub fn delete_word_forward(&mut self) {
        self.edit_each(EditKind::Deleting, |doc, sel| {
            let r = sel.range();
            if !sel.is_empty() {
                return Some((Change::delete(r), 0, None));
            }
            let end = word_right(doc, r.end);
            (end > r.end).then(|| (Change::delete(r.end..end), 0, None))
        });
    }

    /// Replaces a range for every cursor (used by completion): `range_for` gives the
    /// range to replace relative to each cursor. Snippet text is expanded.
    pub fn insert_completion(
        &mut self,
        prefix_len: usize,
        suffix_len: usize,
        text: &str,
        is_snippet: bool,
        extra: Vec<Change>,
    ) {
        // Multi-line snippets: later lines start at this line's indentation, as in
        // other editors.
        let indented;
        let text = if is_snippet && text.contains('\n') {
            let line = self.doc.pos_of(self.sels.primary().head).line;
            let indent: String = self
                .doc
                .line_text(line)
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect();
            indented = text.replace('\n', &format!("\n{indent}"));
            indented.as_str()
        } else {
            text
        };
        let (plain, stops) = if is_snippet {
            let s = Snippet::parse(text);
            (s.text.clone(), Some(s))
        } else {
            (text.to_owned(), None)
        };
        let plain_len = plain.chars().count();
        let first_stop = stops.as_ref().and_then(|s| s.first_stop());
        // The primary cursor's replacement prefix must match at every other cursor.
        let primary = self.sels.primary();
        let prefix = self
            .doc
            .slice(primary.head.saturating_sub(prefix_len)..primary.head);
        // (change, is the completion itself) — extra edits are e.g. auto-imports.
        let mut tagged: Vec<(Change, bool)> = Vec::new();
        for sel in self.sels.iter().copied() {
            let start = sel.head.saturating_sub(prefix_len);
            if self.doc.slice(start..sel.head) != prefix {
                continue;
            }
            let end = (sel.head + suffix_len).min(self.doc.len_chars());
            tagged.push((Change::replace(start..end, plain.clone()), true));
        }
        tagged.extend(extra.into_iter().map(|c| (c, false)));
        tagged.sort_by_key(|(c, _)| c.range.start);
        let mut changes = Vec::new();
        let mut cursors = Vec::new();
        let mut starts = Vec::new();
        let mut delta: isize = 0;
        let mut last_end = 0;
        for (c, main) in tagged {
            if c.range.start < last_end && !changes.is_empty() {
                continue; // overlapping extra edit
            }
            last_end = c.range.end;
            let start_new = (c.range.start as isize + delta) as usize;
            if main {
                let (a, h) = match &first_stop {
                    Some(r) => (start_new + r.start, start_new + r.end),
                    None => (start_new + plain_len, start_new + plain_len),
                };
                cursors.push(Selection::new(a, h));
                starts.push(start_new);
            }
            delta += c.text.chars().count() as isize - c.range.len() as isize;
            changes.push(c);
        }
        if cursors.is_empty() {
            return;
        }
        let after = Selections::new(cursors, 0);
        self.snippet = None;
        self.apply(changes, after, EditKind::Other);
        if let Some(s) = stops
            && s.has_stops()
        {
            self.snippet = Some(SnippetSession::new(&s, &starts));
            if let Some(sels) = self.snippet.as_ref().and_then(|s| s.current_selections()) {
                self.sels = sels;
            }
        }
    }

    /// Tab inside an active snippet: go to the next stop. Returns false if there's none.
    pub fn snippet_next(&mut self, back: bool) -> bool {
        let Some(session) = &mut self.snippet else {
            return false;
        };
        let moved = if back {
            session.retreat()
        } else {
            session.advance()
        };
        match (moved, session.current_selections()) {
            (true, Some(sels)) => {
                self.sels = sels;
                if session.at_end() {
                    self.snippet = None;
                }
                true
            }
            _ => {
                self.snippet = None;
                false
            }
        }
    }

    fn end_snippet_if_outside(&mut self) {
        if let Some(s) = &self.snippet
            && !s.contains(self.sels.primary().head)
        {
            self.snippet = None;
        }
    }

    // ── lines ─────────────────────────────────────────────────────────────

    fn selected_lines(&self) -> Vec<usize> {
        let mut lines = Vec::new();
        for s in self.sels.iter() {
            let r = s.range();
            let first = self.doc.pos_of(r.start).line;
            let mut last = self.doc.pos_of(r.end).line;
            if last > first && self.doc.pos_of(r.end).col == 0 {
                last -= 1;
            }
            for l in first..=last {
                if lines.last() != Some(&l) {
                    lines.push(l);
                }
            }
        }
        lines.dedup();
        lines
    }

    /// Tab: with a multi-line selection indents lines; otherwise inserts spaces to the next stop.
    pub fn tab(&mut self) {
        let multi_line = self.sels.iter().any(|s| {
            let r = s.range();
            self.doc.pos_of(r.start).line != self.doc.pos_of(r.end).line
        });
        if multi_line {
            return self.indent_lines();
        }
        self.edit_each(EditKind::Typing, |doc, sel| {
            let col = doc.pos_of(sel.range().start).col;
            let n = INDENT_WIDTH - col % INDENT_WIDTH;
            Some((Change::replace(sel.range(), " ".repeat(n)), n, None))
        });
    }

    pub fn indent_lines(&mut self) {
        let changes: Vec<Change> = self
            .selected_lines()
            .into_iter()
            .filter(|l| self.doc.line_len(*l) > 0)
            .map(|l| Change::insert(self.doc.rope().line_to_char(l), INDENT))
            .collect();
        let after = self.sels.map(&changes);
        self.apply(changes, after, EditKind::Other);
    }

    pub fn outdent_lines(&mut self) {
        let changes: Vec<Change> = self
            .selected_lines()
            .into_iter()
            .filter_map(|l| {
                let text = self.doc.line_text(l);
                let n = text
                    .chars()
                    .take(INDENT_WIDTH)
                    .take_while(|c| *c == ' ')
                    .count();
                let n = if n == 0 && text.starts_with('\t') {
                    1
                } else {
                    n
                };
                let start = self.doc.rope().line_to_char(l);
                (n > 0).then(|| Change::delete(start..start + n))
            })
            .collect();
        let after = self.sels.map(&changes);
        self.apply(changes, after, EditKind::Other);
    }

    /// Comments or uncomments the selected lines (all commented → uncomment).
    pub fn toggle_comment(&mut self) {
        let Some(token) = self.line_comment else {
            return;
        };
        let lines: Vec<usize> = self
            .selected_lines()
            .into_iter()
            .filter(|l| !self.doc.line_text(*l).trim().is_empty())
            .collect();
        if lines.is_empty() {
            return;
        }
        let all_commented = lines
            .iter()
            .all(|l| self.doc.line_text(*l).trim_start().starts_with(token));
        let min_indent = lines
            .iter()
            .map(|l| {
                self.doc
                    .line_text(*l)
                    .chars()
                    .take_while(|c| *c == ' ')
                    .count()
            })
            .min()
            .unwrap_or(0);
        let changes: Vec<Change> = lines
            .iter()
            .map(|l| {
                let start = self.doc.rope().line_to_char(*l);
                let text = self.doc.line_text(*l);
                if all_commented {
                    let lead = text.chars().take_while(|c| *c == ' ').count();
                    let rest: String = text.chars().skip(lead + token.len()).collect();
                    let n = token.len() + usize::from(rest.starts_with(' '));
                    Change::delete(start + lead..start + lead + n)
                } else {
                    Change::insert(start + min_indent, format!("{token} "))
                }
            })
            .collect();
        let after = self.sels.map(&changes);
        self.apply(changes, after, EditKind::Other);
    }

    // ── movement ──────────────────────────────────────────────────────────

    pub fn set_cursor(&mut self, at: usize, extend: bool) {
        let at = at.min(self.doc.len_chars());
        let primary = self.sels.primary().move_to(at, extend);
        self.sels = Selections::single(primary);
        self.doc.seal_undo();
        self.end_snippet_if_outside();
    }

    pub fn add_cursor(&mut self, at: usize) {
        self.sels = self
            .sels
            .add(Selection::cursor(at.min(self.doc.len_chars())));
        self.doc.seal_undo();
    }

    pub fn select_range(&mut self, range: Range<usize>) {
        let len = self.doc.len_chars();
        self.sels = Selections::single(Selection::new(range.start.min(len), range.end.min(len)));
        self.doc.seal_undo();
    }

    fn move_all(&mut self, extend: bool, f: impl Fn(&Document, Selection) -> usize) {
        let doc = &self.doc;
        self.sels = self.sels.transform(|s| {
            if !extend && !s.is_empty() {
                // Collapse a selection: Left/Right go to its start/end.
                return Selection::cursor(f(doc, s));
            }
            s.move_to(f(doc, s), extend)
        });
        self.doc.seal_undo();
        self.end_snippet_if_outside();
    }

    pub fn move_left(&mut self, extend: bool, word: bool) {
        self.move_all(extend, |d, s| {
            if !extend && !s.is_empty() && !word {
                return s.range().start;
            }
            if word {
                word_left(d, s.head)
            } else {
                s.head.saturating_sub(1)
            }
        });
    }

    pub fn move_right(&mut self, extend: bool, word: bool) {
        self.move_all(extend, |d, s| {
            if !extend && !s.is_empty() && !word {
                return s.range().end;
            }
            if word {
                word_right(d, s.head)
            } else {
                (s.head + 1).min(d.len_chars())
            }
        });
    }

    pub fn move_vertical(&mut self, lines: isize, extend: bool) {
        let doc = &self.doc;
        self.sels = self.sels.transform(|s| {
            let pos = doc.pos_of(s.head);
            let goal = s.goal_col.unwrap_or(pos.col);
            let target = pos.line as isize + lines;
            let head = if target < 0 {
                0
            } else if target as usize >= doc.len_lines() {
                doc.len_chars()
            } else {
                doc.char_of(crate::text::Pos {
                    line: target as usize,
                    col: goal,
                })
            };
            let mut n = s.move_to(head, extend);
            n.goal_col = Some(goal);
            n
        });
        self.doc.seal_undo();
    }

    /// Home: first non-blank char; again: column 0.
    pub fn line_start(&mut self, extend: bool) {
        self.move_all(extend, |d, s| {
            let pos = d.pos_of(s.head);
            let line_start = d.rope().line_to_char(pos.line);
            let text = d.line_text(pos.line);
            let first = text.chars().take_while(|c| c.is_whitespace()).count();
            if pos.col == first {
                line_start
            } else {
                line_start + first
            }
        });
    }

    pub fn line_end(&mut self, extend: bool) {
        self.move_all(extend, |d, s| {
            let line = d.pos_of(s.head).line;
            d.rope().line_to_char(line) + d.line_len(line)
        });
    }

    pub fn doc_start(&mut self, extend: bool) {
        self.move_all(extend, |_, _| 0);
    }

    pub fn doc_end(&mut self, extend: bool) {
        self.move_all(extend, |d, _| d.len_chars());
    }

    pub fn select_all(&mut self) {
        self.sels = Selections::single(Selection::new(0, self.doc.len_chars()));
    }

    /// Escape: back to one cursor.
    pub fn collapse(&mut self) {
        self.snippet = None;
        let p = self.sels.primary();
        self.sels = Selections::single(Selection::cursor(p.head));
    }

    /// ⌘D: select the word at the cursor; again: add the next occurrence.
    pub fn select_next_occurrence(&mut self) {
        let p = self.sels.primary();
        if p.is_empty() {
            let (s, e) = word_at(&self.doc, p.head);
            if s < e {
                self.sels = self
                    .sels
                    .transform(|x| if x == p { Selection::new(s, e) } else { x });
            }
            return;
        }
        let needle = self.doc.slice(p.range());
        let text = self.doc.text();
        let from_byte = self.doc.rope().char_to_byte(p.range().end);
        let found = text[from_byte..]
            .find(&needle)
            .map(|b| from_byte + b)
            .or_else(|| text.find(&needle));
        if let Some(b) = found {
            let s = self.doc.rope().byte_to_char(b);
            let sel = Selection::new(s, s + needle.chars().count());
            if !self.sels.iter().any(|x| x.range() == sel.range()) {
                self.sels = self.sels.add(sel);
            }
        }
    }

    /// Adds a cursor on the line above or below the primary one.
    pub fn add_cursor_vertical(&mut self, down: bool) {
        let p = self.sels.primary();
        let pos = self.doc.pos_of(p.head);
        let goal = p.goal_col.unwrap_or(pos.col);
        let line = if down {
            pos.line + 1
        } else if pos.line == 0 {
            return;
        } else {
            pos.line - 1
        };
        if line >= self.doc.len_lines() {
            return;
        }
        let at = self.doc.char_of(crate::text::Pos { line, col: goal });
        let mut s = Selection::cursor(at);
        s.goal_col = Some(goal);
        self.sels = self.sels.add(s);
    }

    // ── clipboard ─────────────────────────────────────────────────────────

    /// Selected text, one line per selection. With no selection, the whole line.
    pub fn copy(&self) -> String {
        if self.sels.iter().all(Selection::is_empty) {
            let lines: Vec<String> = self
                .sels
                .iter()
                .map(|s| format!("{}\n", self.doc.line_text(self.doc.pos_of(s.head).line)))
                .collect();
            return lines.concat();
        }
        self.sels
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| self.doc.slice(s.range()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn cut(&mut self) -> String {
        let text = self.copy();
        if self.sels.iter().all(Selection::is_empty) {
            // Cut whole lines.
            let changes: Vec<Change> = self
                .sels
                .iter()
                .map(|s| {
                    let line = self.doc.pos_of(s.head).line;
                    let start = self.doc.rope().line_to_char(line);
                    let end = if line + 1 < self.doc.len_lines() {
                        self.doc.rope().line_to_char(line + 1)
                    } else {
                        self.doc.len_chars()
                    };
                    Change::delete(start..end)
                })
                .collect();
            let after = self.sels.map(&changes);
            self.apply(changes, after, EditKind::Other);
        } else {
            self.edit_each(EditKind::Other, |_, s| {
                Some((Change::delete(s.range()), 0, None))
            });
        }
        text
    }

    /// Pastes; if the clipboard has one line per cursor, each cursor gets its own line.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n");
        let lines: Vec<&str> = text.split('\n').collect();
        let per_cursor = self.sels.len() > 1 && lines.len() == self.sels.len();
        let mut i = 0;
        self.edit_each(EditKind::Other, |_, s| {
            let t = if per_cursor {
                lines[i].to_owned()
            } else {
                text.clone()
            };
            i += 1;
            let n = t.chars().count();
            Some((Change::replace(s.range(), t), n, None))
        });
    }

    pub fn undo(&mut self) {
        self.snippet = None;
        if let Some((edits, sels)) = self.doc.undo() {
            self.sels = sels.clamp(self.doc.len_chars());
            self.edits.extend(edits);
        }
    }

    pub fn redo(&mut self) {
        self.snippet = None;
        if let Some((edits, sels)) = self.doc.redo() {
            self.sels = sels.clamp(self.doc.len_chars());
            self.edits.extend(edits);
        }
    }

    /// The identifier being typed just before the primary cursor (for completion).
    pub fn word_before_cursor(&self) -> (usize, String) {
        let head = self.sels.primary().head;
        let mut start = head;
        while start > 0
            && self
                .doc
                .char_at(start - 1)
                .is_some_and(|c| class(c) == Class::Word)
        {
            start -= 1;
        }
        (start, self.doc.slice(start..head))
    }
}

fn word_left(doc: &Document, from: usize) -> usize {
    let mut i = from;
    while i > 0 && doc.char_at(i - 1).is_some_and(|c| class(c) == Class::Space) {
        i -= 1;
    }
    if i == 0 {
        return 0;
    }
    let k = class(doc.char_at(i - 1).unwrap_or(' '));
    if k == Class::Newline {
        return i - 1;
    }
    while i > 0 && doc.char_at(i - 1).is_some_and(|c| class(c) == k) {
        i -= 1;
    }
    i
}

fn word_right(doc: &Document, from: usize) -> usize {
    let len = doc.len_chars();
    let mut i = from;
    while i < len && doc.char_at(i).is_some_and(|c| class(c) == Class::Space) {
        i += 1;
    }
    if i >= len {
        return len;
    }
    let k = class(doc.char_at(i).unwrap_or(' '));
    if k == Class::Newline {
        return i + 1;
    }
    while i < len && doc.char_at(i).is_some_and(|c| class(c) == k) {
        i += 1;
    }
    i
}

/// The word around `at` (identifier chars), or an empty range.
pub fn word_at(doc: &Document, at: usize) -> (usize, usize) {
    let is_word = |i: usize| doc.char_at(i).is_some_and(|c| class(c) == Class::Word);
    let mut s = at;
    while s > 0 && is_word(s - 1) {
        s -= 1;
    }
    let mut e = at;
    while is_word(e) {
        e += 1;
    }
    (s, e)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an editor from text where `|` marks cursors.
    fn ed(marked: &str) -> EditorCore {
        let mut text = String::new();
        let mut cursors = Vec::new();
        for c in marked.chars() {
            if c == '|' {
                cursors.push(text.chars().count());
            } else {
                text.push(c);
            }
        }
        let mut e = EditorCore::new(&text);
        e.sels = Selections::new(cursors.into_iter().map(Selection::cursor).collect(), 0);
        e
    }

    /// Text with `|` at each cursor head.
    fn show(e: &EditorCore) -> String {
        let text = e.text();
        let heads: Vec<usize> = e.sels.iter().map(|s| s.head).collect();
        let mut out = String::new();
        for (i, c) in text.chars().enumerate() {
            if heads.contains(&i) {
                out.push('|');
            }
            out.push(c);
        }
        if heads.contains(&text.chars().count()) {
            out.push('|');
        }
        out
    }

    #[test]
    fn typing_at_multiple_cursors() {
        let mut e = ed("a|\nb|\n");
        e.type_text("x");
        e.type_text("y");
        assert_eq!(show(&e), "axy|\nbxy|\n");
        e.undo();
        assert_eq!(e.text(), "a\nb\n");
    }

    #[test]
    fn auto_pairs_and_overtype() {
        let mut e = ed("let v = |;");
        e.type_text("(");
        assert_eq!(show(&e), "let v = (|);");
        e.type_text("1");
        e.type_text(")");
        assert_eq!(show(&e), "let v = (1)|;");
        let mut e = ed("x|y");
        e.type_text("(");
        assert_eq!(show(&e), "x(|y", "no pair before a word char");
        let mut e = ed("don|");
        e.type_text("\"");
        assert_eq!(show(&e), "don\"|", "no quote pair after a letter");
    }

    #[test]
    fn wrap_selection_in_brackets() {
        let mut e = EditorCore::new("abc");
        e.select_range(0..3);
        e.type_text("[");
        assert_eq!(e.text(), "[abc]");
        assert_eq!(e.sels.primary().range(), 1..4);
    }

    #[test]
    fn smart_newline() {
        let mut e = ed("fn f() {|}");
        e.newline();
        assert_eq!(show(&e), "fn f() {\n    |\n}");
        let mut e = ed("    let x = 1;|");
        e.newline();
        assert_eq!(show(&e), "    let x = 1;\n    |");
    }

    #[test]
    fn backspace_pairs_and_indent() {
        let mut e = ed("(|)");
        e.backspace();
        assert_eq!(show(&e), "|");
        let mut e = ed("        |x");
        e.backspace();
        assert_eq!(show(&e), "    |x");
        let mut e = ed("ab|");
        e.backspace();
        e.backspace();
        e.backspace();
        assert_eq!(show(&e), "|");
        e.undo();
        assert_eq!(e.text(), "ab", "consecutive backspaces undo together");
    }

    #[test]
    fn word_movement_and_delete() {
        let mut e = ed("foo.bar_baz  qux|");
        e.move_left(false, true);
        assert_eq!(show(&e), "foo.bar_baz  |qux");
        e.move_left(false, true);
        assert_eq!(show(&e), "foo.|bar_baz  qux");
        e.delete_word_back();
        assert_eq!(show(&e), "foo|bar_baz  qux");
        e.move_right(false, true);
        assert_eq!(show(&e), "foobar_baz|  qux");
    }

    #[test]
    fn vertical_movement_keeps_goal_column() {
        let mut e = ed("abcdef|\nx\nabcdefgh");
        e.move_vertical(1, false);
        assert_eq!(show(&e), "abcdef\nx|\nabcdefgh");
        e.move_vertical(1, false);
        assert_eq!(show(&e), "abcdef\nx\nabcdef|gh");
    }

    #[test]
    fn home_toggles_between_indent_and_column_zero() {
        let mut e = ed("    foo|");
        e.line_start(false);
        assert_eq!(show(&e), "    |foo");
        e.line_start(false);
        assert_eq!(show(&e), "|    foo");
    }

    #[test]
    fn select_next_occurrence_adds_cursors() {
        let mut e = ed("let a|b = ab + ab;");
        e.select_next_occurrence();
        assert_eq!(e.sels.primary().range(), 4..6);
        e.select_next_occurrence();
        e.select_next_occurrence();
        assert_eq!(e.sels.len(), 3);
        e.type_text("z");
        assert_eq!(e.text(), "let z = z + z;");
    }

    #[test]
    fn indent_outdent_and_comments() {
        let mut e = EditorCore::new("a\nb\n");
        e.select_range(0..3);
        e.indent_lines();
        assert_eq!(e.text(), "    a\n    b\n");
        e.outdent_lines();
        assert_eq!(e.text(), "a\nb\n");
        e.toggle_comment();
        assert_eq!(e.text(), "// a\n// b\n");
        e.toggle_comment();
        assert_eq!(e.text(), "a\nb\n");
    }

    #[test]
    fn clipboard_line_and_per_cursor_paste() {
        let mut e = ed("one|\ntwo\n");
        assert_eq!(e.copy(), "one\n");
        assert_eq!(e.cut(), "one\n");
        assert_eq!(e.text(), "two\n");
        let mut e = ed("a|\nb|\n");
        e.paste("1\n2");
        assert_eq!(show(&e), "a1|\nb2|\n");
    }

    #[test]
    fn completion_with_snippet_and_import() {
        let mut e = ed("fn f() { vec| }");
        e.insert_completion(
            3,
            0,
            "Vec::with_capacity(${1:n})$0",
            true,
            vec![Change::insert(0, "use std::vec::Vec;\n")],
        );
        assert_eq!(
            e.text(),
            "use std::vec::Vec;\nfn f() { Vec::with_capacity(n) }"
        );
        // The placeholder `n` is selected.
        assert_eq!(e.doc.slice(e.sels.primary().range()), "n");
        e.type_text("8");
        assert!(e.snippet_next(false));
        assert_eq!(
            show(&e),
            "use std::vec::Vec;\nfn f() { Vec::with_capacity(8)| }"
        );
        assert!(e.snippet.is_none());
    }

    #[test]
    fn completion_at_multiple_cursors() {
        let mut e = ed("pr|\npr|\n");
        e.insert_completion(2, 0, "println!", false, vec![]);
        assert_eq!(show(&e), "println!|\nprintln!|\n");
    }

    #[test]
    fn multi_line_snippets_follow_the_line_indent() {
        let mut e = ed("fn f() {\n    msg|\n}\n");
        e.insert_completion(3, 0, "if ok(${1:x}) {\n    $0\n}", true, vec![]);
        assert_eq!(e.text(), "fn f() {\n    if ok(x) {\n        \n    }\n}\n");
    }
}
