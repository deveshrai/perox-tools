//! LSP snippets (`$1`, `${1:placeholder}`, `${1|a,b|}`, `$0`) and the Tab-stop session
//! that follows inserting one.

use std::collections::BTreeMap;
use std::ops::Range;

use crate::selection::{Selection, Selections};
use crate::text::{Change, map_pos};

/// A parsed snippet: plain text and tab stops (char ranges into `text`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// Stop number → ranges. `$0` is the final cursor position.
    pub stops: BTreeMap<u32, Vec<Range<usize>>>,
}

impl Snippet {
    pub fn parse(src: &str) -> Self {
        let chars: Vec<char> = src.chars().collect();
        let mut out = Snippet {
            text: String::new(),
            stops: BTreeMap::new(),
        };
        let mut i = 0;
        parse_into(&chars, &mut i, &mut out, false);
        out
    }

    /// The first stop to select: lowest number above 0, else `$0`.
    pub fn first_stop(&self) -> Option<Range<usize>> {
        self.stops
            .iter()
            .find(|(n, _)| **n > 0)
            .or_else(|| self.stops.get_key_value(&0))
            .and_then(|(_, r)| r.first().cloned())
    }

    /// Whether Tab navigation is needed (any stop other than `$0`).
    pub fn has_stops(&self) -> bool {
        self.stops.keys().any(|n| *n > 0)
    }
}

/// Parses until end, or until an unescaped `}` when `nested`.
fn parse_into(chars: &[char], i: &mut usize, out: &mut Snippet, nested: bool) {
    while *i < chars.len() {
        let c = chars[*i];
        match c {
            '\\' if *i + 1 < chars.len()
                && matches!(chars[*i + 1], '$' | '}' | '\\' | ',' | '|') =>
            {
                out.text.push(chars[*i + 1]);
                *i += 2;
            }
            '}' if nested => return,
            '$' => {
                *i += 1;
                parse_dollar(chars, i, out);
            }
            _ => {
                out.text.push(c);
                *i += 1;
            }
        }
    }
}

fn read_number(chars: &[char], i: &mut usize) -> Option<u32> {
    let start = *i;
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        *i += 1;
    }
    chars[start..*i].iter().collect::<String>().parse().ok()
}

fn parse_dollar(chars: &[char], i: &mut usize, out: &mut Snippet) {
    let here = |out: &Snippet| out.text.chars().count();
    if let Some(n) = read_number(chars, i) {
        let at = here(out);
        out.stops.entry(n).or_default().push(at..at);
        return;
    }
    if chars.get(*i) != Some(&'{') {
        // `$NAME` variable: we don't support variables; drop the name.
        while *i < chars.len() && (chars[*i].is_alphanumeric() || chars[*i] == '_') {
            *i += 1;
        }
        return;
    }
    *i += 1; // `{`
    let Some(n) = read_number(chars, i) else {
        // `${NAME:default}` variable: keep the default text.
        while *i < chars.len() && chars[*i] != ':' && chars[*i] != '}' {
            *i += 1;
        }
        if chars.get(*i) == Some(&':') {
            *i += 1;
            parse_into(chars, i, out, true);
        }
        *i += 1;
        return;
    };
    let start = here(out);
    match chars.get(*i) {
        Some(':') => {
            *i += 1;
            parse_into(chars, i, out, true);
            *i += 1; // `}`
        }
        Some('|') => {
            // Choice: use the first option.
            *i += 1;
            let mut first = String::new();
            while *i < chars.len() && chars[*i] != ',' && chars[*i] != '|' {
                first.push(chars[*i]);
                *i += 1;
            }
            while *i < chars.len() && chars[*i] != '}' {
                *i += 1;
            }
            *i += 1;
            out.text.push_str(&first);
        }
        _ => *i += 1, // `}`
    }
    let end = here(out);
    out.stops.entry(n).or_default().push(start..end);
}

/// After inserting a snippet: the stops in document coordinates, which Tab cycles through.
#[derive(Debug, Clone)]
pub struct SnippetSession {
    /// Stops in Tab order; `$0` last. Each holds one range per insertion (multi-cursor)
    /// and per mirror.
    stops: Vec<(u32, Vec<Range<usize>>)>,
    current: usize,
    /// The whole inserted text per insertion, to tell when the cursor has left.
    regions: Vec<Range<usize>>,
}

impl SnippetSession {
    pub fn new(snippet: &Snippet, starts: &[usize]) -> Self {
        let mut order: Vec<u32> = snippet.stops.keys().copied().filter(|n| *n > 0).collect();
        if snippet.stops.contains_key(&0) {
            order.push(0);
        }
        let len = snippet.text.chars().count();
        let stops = order
            .into_iter()
            .map(|n| {
                let ranges = starts
                    .iter()
                    .flat_map(|s| {
                        snippet.stops[&n]
                            .iter()
                            .map(move |r| s + r.start..s + r.end)
                    })
                    .collect();
                (n, ranges)
            })
            .collect();
        Self {
            stops,
            current: 0,
            regions: starts.iter().map(|s| *s..s + len).collect(),
        }
    }

    pub fn map(&mut self, changes: &[Change]) {
        let m = |r: &Range<usize>| map_pos(changes, r.start, false)..map_pos(changes, r.end, true);
        for (_, ranges) in &mut self.stops {
            for r in ranges.iter_mut() {
                *r = m(r);
            }
        }
        for r in &mut self.regions {
            *r = m(r);
        }
    }

    pub fn contains(&self, pos: usize) -> bool {
        self.regions.iter().any(|r| r.start <= pos && pos <= r.end)
    }

    pub fn current_selections(&self) -> Option<Selections> {
        let (_, ranges) = self.stops.get(self.current)?;
        let sels: Vec<Selection> = ranges
            .iter()
            .map(|r| Selection::new(r.start, r.end))
            .collect();
        (!sels.is_empty()).then(|| Selections::new(sels, 0))
    }

    pub fn advance(&mut self) -> bool {
        if self.current + 1 < self.stops.len() {
            self.current += 1;
            true
        } else {
            false
        }
    }

    pub fn retreat(&mut self) -> bool {
        if self.current > 0 {
            self.current -= 1;
            true
        } else {
            false
        }
    }

    /// At `$0` or the last stop: the session ends.
    pub fn at_end(&self) -> bool {
        self.current + 1 >= self.stops.len() || self.stops[self.current].0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stops_placeholders_choices_and_escapes() {
        let s = Snippet::parse("fn ${1:name}(${2:args}) -> ${3|i32,u8|} {\n    $0\n} \\$x");
        assert_eq!(s.text, "fn name(args) -> i32 {\n    \n} $x");
        assert_eq!(s.stops[&1], vec![3..7]);
        assert_eq!(s.stops[&2], vec![8..12]);
        assert_eq!(s.stops[&3], vec![17..20]);
        assert_eq!(s.first_stop(), Some(3..7));
    }

    #[test]
    fn nested_placeholders_and_mirrors() {
        let s = Snippet::parse("${1:a${2:b}c} $1");
        assert_eq!(s.text, "abc ");
        assert_eq!(s.stops[&1], vec![0..3, 4..4]);
        assert_eq!(s.stops[&2], vec![1..2]);
    }

    #[test]
    fn only_final_stop_needs_no_session() {
        let s = Snippet::parse("foo()$0");
        assert!(!s.has_stops());
        assert_eq!(s.first_stop(), Some(5..5));
    }
}
