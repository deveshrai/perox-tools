//! Cursors and selections. There is always at least one; the primary one is where
//! the view scrolls to and where completion appears.

use std::ops::Range;

use crate::text::{Change, map_pos};

/// A selection from `anchor` to `head` (char indices). `head` is the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
    /// Column to aim for when moving up and down through shorter lines.
    pub goal_col: Option<usize>,
}

impl Selection {
    pub fn cursor(at: usize) -> Self {
        Self {
            anchor: at,
            head: at,
            goal_col: None,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Self {
            anchor,
            head,
            goal_col: None,
        }
    }

    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Moves the head; with `extend` the anchor stays, otherwise both move.
    pub fn move_to(self, head: usize, extend: bool) -> Self {
        Self {
            anchor: if extend { self.anchor } else { head },
            head,
            goal_col: None,
        }
    }

    fn map(self, changes: &[Change]) -> Self {
        // A collapsed cursor at an insertion point moves after the inserted text
        // (typing pushes the cursor along). For ranges, keep the inserted text inside.
        if self.is_empty() {
            let p = map_pos(changes, self.head, true);
            return Self::cursor(p);
        }
        let r = self.range();
        let (start, end) = (
            map_pos(changes, r.start, false),
            map_pos(changes, r.end, true),
        );
        if self.anchor <= self.head {
            Self::new(start, end)
        } else {
            Self::new(end, start)
        }
    }
}

/// One or more selections: sorted by position, never overlapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selections {
    ranges: Vec<Selection>,
    primary: usize,
}

impl Selections {
    pub fn single(s: Selection) -> Self {
        Self {
            ranges: vec![s],
            primary: 0,
        }
    }

    /// Normalises: sorts, merges overlapping or touching-and-empty selections.
    pub fn new(ranges: Vec<Selection>, primary: usize) -> Self {
        assert!(!ranges.is_empty(), "at least one selection");
        let primary_sel = ranges[primary.min(ranges.len() - 1)];
        let mut indexed: Vec<Selection> = ranges;
        indexed.sort_by_key(|s| (s.range().start, s.range().end));
        let mut merged: Vec<Selection> = Vec::with_capacity(indexed.len());
        let mut primary_idx = 0;
        for s in indexed {
            if let Some(last) = merged.last_mut() {
                let (lr, sr) = (last.range(), s.range());
                // Overlapping ranges merge; so do identical cursors. Adjacent ranges don't.
                let overlaps =
                    sr.start < lr.end || (sr.start == lr.end && s.is_empty() && last.is_empty());
                if overlaps {
                    let start = lr.start.min(sr.start);
                    let end = lr.end.max(sr.end);
                    let forward = last.anchor <= last.head;
                    *last = if forward {
                        Selection::new(start, end)
                    } else {
                        Selection::new(end, start)
                    };
                    if s == primary_sel {
                        primary_idx = merged.len() - 1;
                    }
                    continue;
                }
            }
            if s == primary_sel {
                primary_idx = merged.len();
            }
            merged.push(s);
        }
        Self {
            ranges: merged,
            primary: primary_idx,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Selection> {
        self.ranges.iter()
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn primary(&self) -> Selection {
        self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    /// Applies `f` to every selection and normalises.
    pub fn transform(&self, f: impl FnMut(Selection) -> Selection) -> Self {
        Self::new(self.ranges.iter().copied().map(f).collect(), self.primary)
    }

    /// Maps through edits (changes in pre-edit coordinates, ascending).
    pub fn map(&self, changes: &[Change]) -> Self {
        self.transform(|s| s.map(changes))
    }

    /// Adds a selection and makes it primary.
    pub fn add(&self, s: Selection) -> Self {
        let mut ranges = self.ranges.clone();
        ranges.push(s);
        let n = ranges.len() - 1;
        Self::new(ranges, n)
    }

    /// Keeps only the primary selection.
    pub fn only_primary(&self) -> Self {
        Self::single(self.primary())
    }

    pub fn clamp(&self, len: usize) -> Self {
        self.transform(|s| Selection {
            anchor: s.anchor.min(len),
            head: s.head.min(len),
            goal_col: s.goal_col,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_overlapping_and_duplicate_cursors() {
        let s = Selections::new(
            vec![
                Selection::new(0, 5),
                Selection::new(3, 8),
                Selection::cursor(10),
                Selection::cursor(10),
            ],
            2,
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s.iter().next().unwrap().range(), 0..8);
        assert_eq!(s.primary(), Selection::cursor(10));
    }

    #[test]
    fn adjacent_ranges_stay_separate() {
        let s = Selections::new(vec![Selection::new(0, 3), Selection::new(3, 6)], 0);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn cursors_move_with_typing() {
        let s = Selections::new(vec![Selection::cursor(1), Selection::cursor(4)], 0);
        let changes = vec![Change::insert(1, "x"), Change::insert(4, "x")];
        let m = s.map(&changes);
        let heads: Vec<_> = m.iter().map(|s| s.head).collect();
        assert_eq!(heads, [2, 6]);
    }
}
