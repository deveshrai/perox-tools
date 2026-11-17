//! The text buffer: a rope, edits expressed as [`Change`]s, undo/redo, and the
//! per-change reports that tree-sitter and LSP servers need.
//!
//! Positions are **char indices** into the whole document. LSP positions use
//! `(line, char column)`: we negotiate the `utf-32` position encoding with
//! rust-analyzer, so a column is a char count and no UTF-16 conversion is needed.

use std::ops::Range;
use std::time::{Duration, Instant};

use ropey::Rope;

use crate::selection::{Selection, Selections};

/// Replace `range` (char indices in the text *before* the transaction) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub range: Range<usize>,
    pub text: String,
}

impl Change {
    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self {
            range: at..at,
            text: text.into(),
        }
    }

    pub fn delete(range: Range<usize>) -> Self {
        Self {
            range,
            text: String::new(),
        }
    }

    pub fn replace(range: Range<usize>, text: impl Into<String>) -> Self {
        Self {
            range,
            text: text.into(),
        }
    }
}

/// `(line, column)`, both zero-based; the column counts chars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

/// One applied change, described in the coordinates of the document just before it
/// was applied: the form LSP's incremental sync and tree-sitter's `InputEdit` want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    pub start: Pos,
    pub old_end: Pos,
    pub new_text: String,
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    /// tree-sitter points: (row, byte column).
    pub start_point: (usize, usize),
    pub old_end_point: (usize, usize),
    pub new_end_point: (usize, usize),
}

/// What kind of edit, for grouping undo steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Typing: consecutive typing merges into one undo step.
    Typing,
    /// Deleting with Backspace/Delete: consecutive deletes merge.
    Deleting,
    /// Anything else: always its own undo step.
    Other,
}

#[derive(Debug, Clone)]
struct Step {
    /// Transactions that undo this step, applied in order (newest first).
    undo_chain: Vec<Vec<Change>>,
    /// Transactions that redo this step, applied in order (oldest first).
    redo_chain: Vec<Vec<Change>>,
    before: Selections,
    after: Selections,
    kind: EditKind,
    at: Instant,
    revision: u64,
    parent_revision: u64,
}

/// A text document with history.
#[derive(Debug, Clone)]
pub struct Document {
    rope: Rope,
    /// Increases with every change, including undo and redo (LSP `version`).
    version: i32,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Identifies the current content state; equal revisions mean equal text.
    revision: u64,
    next_revision: u64,
    saved_revision: u64,
    /// Stop merging typing into the previous step.
    seal: bool,
}

const MERGE_WINDOW: Duration = Duration::from_millis(1500);

impl Document {
    pub fn new(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            version: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            next_revision: 1,
            saved_revision: 0,
            seal: false,
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
        self.seal = true;
    }

    /// The next edit starts a new undo step (call on cursor jumps, focus loss…).
    pub fn seal_undo(&mut self) {
        self.seal = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    // ── positions ─────────────────────────────────────────────────────────

    pub fn pos_of(&self, char_idx: usize) -> Pos {
        let idx = char_idx.min(self.rope.len_chars());
        let line = self.rope.char_to_line(idx);
        Pos {
            line,
            col: idx - self.rope.line_to_char(line),
        }
    }

    /// Clamps to the end of the line and of the document.
    pub fn char_of(&self, pos: Pos) -> usize {
        if pos.line >= self.rope.len_lines() {
            return self.rope.len_chars();
        }
        self.rope.line_to_char(pos.line) + pos.col.min(self.line_len(pos.line))
    }

    /// Length of a line in chars, without its line ending.
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }

    /// A line's text without its line ending.
    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let start = self.rope.line_to_char(line);
        self.rope
            .slice(start..start + self.line_len(line))
            .to_string()
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.rope.len_chars());
        let start = range.start.min(end);
        self.rope.slice(start..end).to_string()
    }

    pub fn char_at(&self, idx: usize) -> Option<char> {
        (idx < self.rope.len_chars()).then(|| self.rope.char(idx))
    }

    // ── editing ───────────────────────────────────────────────────────────

    /// Applies changes (any order; they must not overlap) and records an undo step.
    /// `after` computes the new selections from the old ones mapped through the changes.
    pub fn apply(
        &mut self,
        mut changes: Vec<Change>,
        before: &Selections,
        after: Selections,
        kind: EditKind,
    ) -> Vec<Edited> {
        changes.retain(|c| !(c.range.is_empty() && c.text.is_empty()));
        if changes.is_empty() {
            return Vec::new();
        }
        changes.sort_by_key(|c| c.range.start);
        let (edited, inverse) = self.apply_raw(&changes);

        let now = Instant::now();
        let merge = !self.seal
            && kind != EditKind::Other
            && self.redo.is_empty()
            && self.undo.last().is_some_and(|last| {
                last.kind == kind
                    && last.revision == self.revision
                    && now.duration_since(last.at) < MERGE_WINDOW
                    && !(kind == EditKind::Typing && changes.iter().any(|c| c.text.contains('\n')))
            });
        let parent = self.revision;
        self.revision = self.next_revision;
        self.next_revision += 1;
        if merge {
            let last = self.undo.last_mut().expect("checked above");
            last.undo_chain.insert(0, inverse);
            last.redo_chain.push(changes);
            last.after = after;
            last.at = now;
            last.revision = self.revision;
        } else {
            self.undo.push(Step {
                undo_chain: vec![inverse],
                redo_chain: vec![changes],
                before: before.clone(),
                after,
                kind,
                at: now,
                revision: self.revision,
                parent_revision: parent,
            });
        }
        self.redo.clear();
        self.seal = false;
        edited
    }

    /// Undoes one step; returns the edits and the selections to restore.
    pub fn undo(&mut self) -> Option<(Vec<Edited>, Selections)> {
        let step = self.undo.pop()?;
        let mut edited = Vec::new();
        for tx in &step.undo_chain {
            edited.extend(self.apply_raw(tx).0);
        }
        self.revision = step.parent_revision;
        self.seal = true;
        let before = step.before.clone();
        self.redo.push(step);
        Some((edited, before))
    }

    /// Redoes one step; returns the edits and the selections to restore.
    pub fn redo(&mut self) -> Option<(Vec<Edited>, Selections)> {
        let step = self.redo.pop()?;
        let mut edited = Vec::new();
        for tx in &step.redo_chain {
            edited.extend(self.apply_raw(tx).0);
        }
        self.revision = step.revision;
        self.seal = true;
        let after = step.after.clone();
        self.undo.push(step);
        Some((edited, after))
    }

    /// Applies non-overlapping changes sorted ascending, given in pre-transaction
    /// coordinates. Returns the edits and the inverse (in post-transaction coordinates,
    /// sorted ascending, so they can themselves be applied with this function).
    fn apply_raw(&mut self, changes: &[Change]) -> (Vec<Edited>, Vec<Change>) {
        let mut edited = Vec::with_capacity(changes.len());
        let mut inverse = Vec::with_capacity(changes.len());
        let mut delta: isize = 0;
        for c in changes {
            let start = (c.range.start as isize + delta) as usize;
            let end = (c.range.end as isize + delta) as usize;
            let (e, old) = self.splice(start, end, &c.text);
            edited.push(e);
            let new_len = c.text.chars().count();
            inverse.push(Change::replace(start..start + new_len, old));
            delta += new_len as isize - (end - start) as isize;
        }
        self.version += 1;
        (edited, inverse)
    }

    /// Replaces `start..end` (current coordinates) and reports the edit.
    fn splice(&mut self, start: usize, end: usize, text: &str) -> (Edited, String) {
        let len = self.rope.len_chars();
        let (start, end) = (start.min(len), end.min(len).max(start.min(len)));
        let start_pos = self.pos_of(start);
        let old_end_pos = self.pos_of(end);
        let start_byte = self.rope.char_to_byte(start);
        let old_end_byte = self.rope.char_to_byte(end);
        let start_point = self.point_of(start);
        let old_end_point = self.point_of(end);
        let old = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        self.rope.insert(start, text);
        let new_end = start + text.chars().count();
        let edited = Edited {
            start: start_pos,
            old_end: old_end_pos,
            new_text: text.to_owned(),
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + text.len(),
            start_point,
            old_end_point,
            new_end_point: self.point_of(new_end),
        };
        (edited, old)
    }

    /// tree-sitter point: (row, byte column).
    fn point_of(&self, char_idx: usize) -> (usize, usize) {
        let line = self.rope.char_to_line(char_idx);
        let line_byte = self.rope.line_to_byte(line);
        (line, self.rope.char_to_byte(char_idx) - line_byte)
    }

    /// Replaces the whole text (reload from disk). One undo step.
    pub fn replace_all(&mut self, text: &str, sels: &Selections) -> Vec<Edited> {
        let len = self.len_chars();
        let after = Selections::single(Selection::cursor(
            sels.primary().head.min(text.chars().count()),
        ));
        self.apply(
            vec![Change::replace(0..len, text)],
            sels,
            after,
            EditKind::Other,
        )
    }
}

/// Maps a position through changes given in pre-transaction coordinates (ascending).
/// `after_insert`: a position exactly at the start of a change moves past the new text.
pub fn map_pos(changes: &[Change], pos: usize, after_insert: bool) -> usize {
    let mut delta: isize = 0;
    for c in changes {
        let (s, e) = (c.range.start, c.range.end);
        let len = c.text.chars().count() as isize;
        if pos < s {
            break;
        }
        if pos == s {
            return (s as isize + delta + if after_insert { len } else { 0 }) as usize;
        }
        if pos < e {
            return (s as isize + delta + len) as usize;
        }
        delta += len - (e - s) as isize;
    }
    (pos as isize + delta).max(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sels(at: usize) -> Selections {
        Selections::single(Selection::cursor(at))
    }

    #[test]
    fn positions_and_lines() {
        let d = Document::new("ab\ncdé\r\n\nx");
        assert_eq!(d.len_lines(), 4);
        assert_eq!(d.line_len(1), 3);
        assert_eq!(d.line_text(1), "cdé");
        assert_eq!(d.pos_of(5), Pos { line: 1, col: 2 });
        assert_eq!(d.char_of(Pos { line: 1, col: 99 }), 6);
        assert_eq!(d.char_of(Pos { line: 9, col: 0 }), d.len_chars());
    }

    #[test]
    fn multi_change_apply_reports_sequential_edits() {
        let mut d = Document::new("aXbXc");
        let edits = d.apply(
            vec![Change::replace(1..2, "11"), Change::replace(3..4, "")],
            &sels(0),
            sels(0),
            EditKind::Other,
        );
        assert_eq!(d.text(), "a11bc");
        // Second edit is in coordinates after the first was applied.
        assert_eq!(edits[1].start, Pos { line: 0, col: 4 });
        assert_eq!(edits[1].old_end, Pos { line: 0, col: 5 });
        assert_eq!(d.version(), 1);
        assert!(d.is_dirty());
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut d = Document::new("hello world");
        d.apply(
            vec![Change::replace(0..5, "goodbye")],
            &sels(0),
            sels(7),
            EditKind::Other,
        );
        d.apply(
            vec![Change::insert(13, "!"), Change::delete(7..8)],
            &sels(7),
            sels(7),
            EditKind::Other,
        );
        assert_eq!(d.text(), "goodbyeworld!");
        d.undo().unwrap();
        assert_eq!(d.text(), "goodbye world");
        d.undo().unwrap();
        assert_eq!(d.text(), "hello world");
        assert!(!d.is_dirty());
        assert!(d.undo().is_none());
        d.redo().unwrap();
        d.redo().unwrap();
        assert_eq!(d.text(), "goodbyeworld!");
        d.undo().unwrap();
        assert_eq!(d.text(), "goodbye world");
    }

    #[test]
    fn typing_merges_into_one_undo_step() {
        let mut d = Document::new("");
        for (i, ch) in "fn main".chars().enumerate() {
            d.apply(
                vec![Change::insert(i, ch.to_string())],
                &sels(i),
                sels(i + 1),
                EditKind::Typing,
            );
        }
        assert_eq!(d.text(), "fn main");
        d.undo().unwrap();
        assert_eq!(d.text(), "");
        d.redo().unwrap();
        assert_eq!(d.text(), "fn main");
    }

    #[test]
    fn typing_with_multiple_cursors_merges_and_undoes() {
        let mut d = Document::new("a\nb\n");
        // Two cursors at line ends (1 and 3); type "xy".
        let two = |a, b| Selections::new(vec![Selection::cursor(a), Selection::cursor(b)], 0);
        d.apply(
            vec![Change::insert(1, "x"), Change::insert(3, "x")],
            &two(1, 3),
            two(2, 5),
            EditKind::Typing,
        );
        d.apply(
            vec![Change::insert(2, "y"), Change::insert(5, "y")],
            &two(2, 5),
            two(3, 7),
            EditKind::Typing,
        );
        assert_eq!(d.text(), "axy\nbxy\n");
        d.undo().unwrap();
        assert_eq!(d.text(), "a\nb\n");
    }

    #[test]
    fn newline_breaks_typing_merge_and_save_clears_dirty() {
        let mut d = Document::new("");
        d.apply(
            vec![Change::insert(0, "a")],
            &sels(0),
            sels(1),
            EditKind::Typing,
        );
        d.apply(
            vec![Change::insert(1, "\n")],
            &sels(1),
            sels(2),
            EditKind::Typing,
        );
        d.mark_saved();
        assert!(!d.is_dirty());
        d.undo().unwrap();
        assert_eq!(d.text(), "a");
        assert!(d.is_dirty());
        d.redo().unwrap();
        assert!(!d.is_dirty());
    }

    #[test]
    fn map_positions() {
        let ch = vec![Change::insert(2, "abc"), Change::replace(5..7, "z")];
        assert_eq!(map_pos(&ch, 0, true), 0);
        assert_eq!(map_pos(&ch, 2, true), 5);
        assert_eq!(map_pos(&ch, 2, false), 2);
        assert_eq!(map_pos(&ch, 4, true), 7);
        assert_eq!(map_pos(&ch, 6, true), 9);
        assert_eq!(map_pos(&ch, 9, true), 11);
    }

    #[test]
    fn edited_reports_bytes_and_points_for_tree_sitter() {
        let mut d = Document::new("é\nab");
        let e = d.apply(
            vec![Change::insert(3, "Ω")],
            &sels(0),
            sels(0),
            EditKind::Other,
        );
        assert_eq!(e[0].start_byte, 4); // é is 2 bytes, \n 1, a 1
        assert_eq!(e[0].start_point, (1, 1));
        assert_eq!(e[0].new_end_byte, 6);
        assert_eq!(e[0].new_end_point, (1, 3));
    }
}
