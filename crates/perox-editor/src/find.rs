//! Find and replace within one document.

use std::ops::Range;

use crate::core::EditorCore;
use crate::selection::{Selection, Selections};
use crate::text::{Change, EditKind};

const MAX_MATCHES: usize = 10_000;

#[derive(Debug, Clone, Default)]
pub struct Find {
    pub open: bool,
    pub query: String,
    pub replacement: String,
    pub show_replace: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Char ranges of all matches, in order.
    pub matches: Vec<Range<usize>>,
    /// Index of the current match.
    pub current: Option<usize>,
    /// Why the pattern is invalid, if it is.
    pub error: Option<String>,
    /// (doc version, query, flags) the matches were computed for.
    computed_for: Option<(i32, String, bool, bool, bool)>,
    /// Focus the query field on the next frame.
    pub focus_query: bool,
}

impl Find {
    fn pattern(&self) -> Result<regex::Regex, String> {
        let mut p = if self.regex {
            self.query.clone()
        } else {
            regex::escape(&self.query)
        };
        if self.whole_word {
            p = format!(r"\b(?:{p})\b");
        }
        regex::RegexBuilder::new(&p)
            .case_insensitive(!self.case_sensitive)
            .multi_line(true)
            .build()
            .map_err(|e| {
                e.to_string()
                    .lines()
                    .last()
                    .unwrap_or("invalid pattern")
                    .to_owned()
            })
    }

    /// Recomputes matches if the text or query changed. Picks the match at or after `from`.
    pub fn update(&mut self, core: &EditorCore, from: usize) {
        let key = (
            core.doc.version(),
            self.query.clone(),
            self.case_sensitive,
            self.whole_word,
            self.regex,
        );
        if self.computed_for.as_ref() == Some(&key) {
            return;
        }
        self.computed_for = Some(key);
        self.matches.clear();
        self.error = None;
        if self.query.is_empty() {
            self.current = None;
            return;
        }
        let re = match self.pattern() {
            Ok(re) => re,
            Err(e) => {
                self.error = Some(e);
                self.current = None;
                return;
            }
        };
        let text = core.doc.text();
        let rope = core.doc.rope();
        for m in re
            .find_iter(&text)
            .filter(|m| !m.is_empty())
            .take(MAX_MATCHES)
        {
            self.matches
                .push(rope.byte_to_char(m.start())..rope.byte_to_char(m.end()));
        }
        self.current = self
            .matches
            .iter()
            .position(|r| r.start >= from)
            .or((!self.matches.is_empty()).then_some(0));
    }

    /// Moves to the next (or previous) match and selects it.
    pub fn step(&mut self, core: &mut EditorCore, back: bool) {
        self.update(core, core.sels.primary().head);
        let n = self.matches.len();
        if n == 0 {
            return;
        }
        let head = core.sels.primary().range();
        let i = if back {
            self.matches
                .iter()
                .rposition(|r| r.start < head.start)
                .unwrap_or(n - 1)
        } else {
            self.matches
                .iter()
                .position(|r| r.start > head.start)
                .unwrap_or(0)
        };
        self.current = Some(i);
        core.select_range(self.matches[i].clone());
    }

    /// Selects every match as a cursor (⌥Enter): edit them all at once.
    pub fn select_all(&mut self, core: &mut EditorCore) {
        self.update(core, 0);
        if self.matches.is_empty() {
            return;
        }
        let sels: Vec<Selection> = self
            .matches
            .iter()
            .map(|r| Selection::new(r.start, r.end))
            .collect();
        core.sels = Selections::new(sels, self.current.unwrap_or(0));
    }

    fn replacement_for(&self, matched: &str) -> String {
        if self.regex
            && let Ok(re) = self.pattern()
        {
            return re.replace(matched, self.replacement.as_str()).into_owned();
        }
        self.replacement.clone()
    }

    /// Replaces the current match and moves to the next one.
    pub fn replace_one(&mut self, core: &mut EditorCore) {
        self.update(core, core.sels.primary().head);
        let Some(i) = self.current else { return };
        let Some(r) = self.matches.get(i).cloned() else {
            return;
        };
        let text = self.replacement_for(&core.doc.slice(r.clone()));
        let len = text.chars().count();
        let after = Selections::single(Selection::cursor(r.start + len));
        core.apply(vec![Change::replace(r, text)], after, EditKind::Other);
        self.update(core, core.sels.primary().head);
        if let Some(i) = self.current {
            core.select_range(self.matches[i].clone());
        }
    }

    /// Replaces every match as one undo step. Returns how many.
    pub fn replace_all(&mut self, core: &mut EditorCore) -> usize {
        self.update(core, 0);
        let changes: Vec<Change> = self
            .matches
            .iter()
            .map(|r| Change::replace(r.clone(), self.replacement_for(&core.doc.slice(r.clone()))))
            .collect();
        let n = changes.len();
        let after = core.sels.map(&changes);
        core.apply(changes, after, EditKind::Other);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(q: &str) -> Find {
        Find {
            query: q.into(),
            ..Default::default()
        }
    }

    #[test]
    fn finds_case_insensitively_by_default() {
        let core = EditorCore::new("Foo foo FOO food");
        let mut f = find("foo");
        f.update(&core, 0);
        assert_eq!(f.matches.len(), 4);
        f.whole_word = true;
        f.update(&core, 0);
        assert_eq!(f.matches.len(), 3);
        f.case_sensitive = true;
        f.update(&core, 0);
        assert_eq!(f.matches, vec![4..7]);
    }

    #[test]
    fn steps_and_wraps() {
        let mut core = EditorCore::new("a x a x a");
        let mut f = find("a");
        f.step(&mut core, false);
        assert_eq!(core.sels.primary().range(), 4..5);
        f.step(&mut core, false);
        f.step(&mut core, false);
        assert_eq!(core.sels.primary().range(), 0..1, "wraps around");
        f.step(&mut core, true);
        assert_eq!(core.sels.primary().range(), 8..9);
    }

    #[test]
    fn replace_all_with_regex_groups_is_one_undo_step() {
        let mut core = EditorCore::new("let a = 1;\nlet b = 2;");
        let mut f = Find {
            query: r"let (\w+)".into(),
            replacement: "let mut $1".into(),
            regex: true,
            ..Default::default()
        };
        assert_eq!(f.replace_all(&mut core), 2);
        assert_eq!(core.text(), "let mut a = 1;\nlet mut b = 2;");
        core.undo();
        assert_eq!(core.text(), "let a = 1;\nlet b = 2;");
    }

    #[test]
    fn invalid_regex_reports_an_error() {
        let core = EditorCore::new("x");
        let mut f = Find {
            query: "(".into(),
            regex: true,
            ..Default::default()
        };
        f.update(&core, 0);
        assert!(f.error.is_some());
    }

    #[test]
    fn select_all_matches_makes_cursors() {
        let mut core = EditorCore::new("ab ab ab");
        let mut f = find("ab");
        f.select_all(&mut core);
        core.type_text("z");
        assert_eq!(core.text(), "z z z");
    }
}
