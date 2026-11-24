//! Syntax highlighting with tree-sitter: incremental parsing after each edit, and the
//! highlight query run only over the lines on screen.
//! rust-analyzer's semantic tokens are layered on top by the view (see `Decorations`).

use std::ops::Range;
use std::sync::OnceLock;

use ropey::Rope;
use tree_sitter::{
    InputEdit, Language, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree,
};

use crate::text::Edited;

/// What a span of text is, for colouring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hl {
    Keyword,
    Function,
    Method,
    Macro,
    Type,
    BuiltinType,
    Constant,
    Number,
    String,
    Escape,
    Comment,
    DocComment,
    Attribute,
    Property,
    Label,
    Operator,
    Punctuation,
    Variable,
    Parameter,
    SelfKw,
    Lifetime,
    Namespace,
    Mutable,
    Unsafe,
}

impl Hl {
    /// Maps a tree-sitter capture name (`function.method`) to a highlight.
    pub fn from_capture(name: &str) -> Option<Self> {
        Some(match name {
            "keyword" => Hl::Keyword,
            "function" => Hl::Function,
            "function.method" => Hl::Method,
            "function.macro" => Hl::Macro,
            "type" | "constructor" => Hl::Type,
            "type.builtin" => Hl::BuiltinType,
            "constant" => Hl::Constant,
            "constant.builtin" => Hl::Number,
            "string" => Hl::String,
            "escape" => Hl::Escape,
            "comment" => Hl::Comment,
            "comment.documentation" => Hl::DocComment,
            "attribute" => Hl::Attribute,
            "property" => Hl::Property,
            "label" => Hl::Label,
            "operator" => Hl::Operator,
            "punctuation.bracket" | "punctuation.delimiter" => Hl::Punctuation,
            "variable.parameter" => Hl::Parameter,
            "variable.builtin" => Hl::SelfKw,
            _ => return None,
        })
    }
}

/// Languages the editor knows. Others are plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Slint,
    Plain,
}

impl Lang {
    pub fn from_path(path: &std::path::Path) -> Self {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => Lang::Rust,
            Some("slint") => Lang::Slint,
            _ => Lang::Plain,
        }
    }
}

struct Highlighter {
    language: Language,
    query: Query,
    /// Capture index → highlight.
    map: Vec<Option<Hl>>,
}

fn rust() -> &'static Highlighter {
    static RUST: OnceLock<Highlighter> = OnceLock::new();
    RUST.get_or_init(|| {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let query = Query::new(&language, tree_sitter_rust::HIGHLIGHTS_QUERY)
            .expect("bundled Rust highlight query compiles");
        let map = query
            .capture_names()
            .iter()
            .map(|n| Hl::from_capture(n))
            .collect();
        Highlighter {
            language,
            query,
            map,
        }
    })
}

fn slint() -> &'static Highlighter {
    static SLINT: OnceLock<Highlighter> = OnceLock::new();
    SLINT.get_or_init(|| {
        let language: Language = tree_sitter_slint::LANGUAGE.into();
        let query = Query::new(&language, include_str!("../queries/slint-highlights.scm"))
            .expect("bundled Slint highlight query compiles");
        let map = query
            .capture_names()
            .iter()
            .map(|n| Hl::from_capture(n))
            .collect();
        Highlighter {
            language,
            query,
            map,
        }
    })
}

/// A parse tree kept in step with a document.
pub struct Syntax {
    parser: Parser,
    tree: Option<Tree>,
    hl: &'static Highlighter,
}

impl std::fmt::Debug for Syntax {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Syntax").finish_non_exhaustive()
    }
}

impl Syntax {
    pub fn new(lang: Lang, rope: &Rope) -> Option<Self> {
        let hl = match lang {
            Lang::Rust => rust(),
            Lang::Slint => slint(),
            Lang::Plain => return None,
        };
        let mut parser = Parser::new();
        parser.set_language(&hl.language).ok()?;
        let mut s = Self {
            parser,
            tree: None,
            hl,
        };
        s.reparse(rope);
        Some(s)
    }

    /// Tells the tree about edits (in order), then reparses incrementally.
    pub fn update(&mut self, edits: &[Edited], rope: &Rope) {
        if edits.is_empty() {
            return;
        }
        if let Some(tree) = &mut self.tree {
            for e in edits {
                tree.edit(&InputEdit {
                    start_byte: e.start_byte,
                    old_end_byte: e.old_end_byte,
                    new_end_byte: e.new_end_byte,
                    start_position: Point::new(e.start_point.0, e.start_point.1),
                    old_end_position: Point::new(e.old_end_point.0, e.old_end_point.1),
                    new_end_position: Point::new(e.new_end_point.0, e.new_end_point.1),
                });
            }
        }
        self.reparse(rope);
    }

    fn reparse(&mut self, rope: &Rope) {
        let len = rope.len_bytes();
        let mut read = |byte: usize, _: Point| -> &[u8] {
            if byte >= len {
                return &[];
            }
            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - start..]
        };
        self.tree = self
            .parser
            .parse_with_options(&mut read, self.tree.as_ref(), None);
    }

    /// Highlight spans (byte ranges) intersecting `bytes`, sorted by start. Where
    /// captures overlap, the innermost (shortest) wins.
    pub fn highlights(&self, rope: &Rope, bytes: Range<usize>) -> Vec<(Range<usize>, Hl)> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes);
        let text = |node: Node<'_>| {
            let r = node.byte_range();
            let end = r.end.min(rope.len_bytes());
            rope.byte_slice(r.start.min(end)..end)
                .chunks()
                .map(str::as_bytes)
        };
        let mut spans: Vec<(Range<usize>, Hl)> = Vec::new();
        let mut captures = cursor.captures(&self.hl.query, tree.root_node(), text);
        while let Some((m, idx)) = captures.next() {
            let cap = m.captures()[*idx];
            if let Some(Some(hl)) = self.hl.map.get(cap.index as usize) {
                spans.push((cap.node.byte_range(), *hl));
            }
        }
        // Lifetimes are `'a` labels in this grammar's query: recolour them.
        for (r, hl) in &mut spans {
            if *hl == Hl::Label && rope.byte_slice(r.clone()).chars().next() == Some('\'') {
                *hl = Hl::Lifetime;
            }
        }
        // Innermost first: sort by start, then by length so later passes can override.
        spans.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
        spans
    }

    /// Smallest node kinds around a byte offset (for "in string or comment" checks).
    pub fn is_in_comment_or_string(&self, byte: usize) -> bool {
        let Some(tree) = &self.tree else { return false };
        let Some(node) = tree
            .root_node()
            .descendant_for_byte_range(byte.saturating_sub(1), byte)
        else {
            return false;
        };
        let mut n = Some(node);
        while let Some(x) = n {
            let k = x.kind();
            if k.contains("comment")
                || k == "string_literal"
                || k == "raw_string_literal"
                || k == "char_literal"
            {
                return true;
            }
            n = x.parent();
        }
        false
    }
}

/// Flattens possibly-nested spans into non-overlapping char-column sections for one
/// line: later (inner) spans override earlier (outer) ones.
pub fn line_sections(
    spans: &[(Range<usize>, Hl)],
    line_start_byte: usize,
    line_text: &str,
) -> Vec<(Range<usize>, Hl)> {
    let n_bytes = line_text.len();
    let mut per_byte: Vec<Option<Hl>> = vec![None; n_bytes];
    let line_end = line_start_byte + n_bytes;
    for (r, hl) in spans {
        if r.end <= line_start_byte || r.start >= line_end {
            continue;
        }
        let s = r.start.max(line_start_byte) - line_start_byte;
        let e = r.end.min(line_end) - line_start_byte;
        for b in &mut per_byte[s..e] {
            *b = Some(*hl);
        }
    }
    // Convert byte runs to char columns.
    let mut out: Vec<(Range<usize>, Hl)> = Vec::new();
    for (col, (b, _)) in line_text.char_indices().enumerate() {
        if let Some(hl) = per_byte[b] {
            match out.last_mut() {
                Some((r, h)) if *h == hl && r.end == col => r.end = col + 1,
                _ => out.push((col..col + 1, hl)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::{Selection, Selections};
    use crate::text::{Change, Document, EditKind};

    fn kinds(src: &str) -> Vec<(String, Hl)> {
        kinds_in(Lang::Rust, src)
    }

    fn kinds_in(lang: Lang, src: &str) -> Vec<(String, Hl)> {
        let rope = Rope::from_str(src);
        let s = Syntax::new(lang, &rope).unwrap();
        let spans = s.highlights(&rope, 0..rope.len_bytes());
        let mut out = Vec::new();
        for (line_no, line) in src.lines().enumerate() {
            let start = rope.line_to_byte(line_no);
            for (r, hl) in line_sections(&spans, start, line) {
                out.push((line.chars().skip(r.start).take(r.len()).collect(), hl));
            }
        }
        out
    }

    #[test]
    fn highlights_slint() {
        let k = kinds_in(
            Lang::Slint,
            "import { Button } from \"std-widgets.slint\";\n// note\nexport component Main inherits Window {\n    in-out property <string> name: \"x\";\n    callback go();\n    ok := Button { text: \"OK\"; width: 12px; clicked => { root.go(); } }\n}\n",
        );
        let find = |t: &str| k.iter().find(|(s, _)| s == t).map(|(_, h)| *h);
        assert_eq!(find("import"), Some(Hl::Keyword), "{k:?}");
        assert_eq!(find("component"), Some(Hl::Keyword));
        assert_eq!(find("Main"), Some(Hl::Type));
        assert_eq!(find("Button"), Some(Hl::Type));
        assert_eq!(find("// note"), Some(Hl::Comment));
        assert_eq!(find("\"OK\""), Some(Hl::String));
        assert_eq!(find("12px"), Some(Hl::Number));
        assert_eq!(find("text"), Some(Hl::Property));
        assert_eq!(find("go"), Some(Hl::Function));
        assert_eq!(find("root"), Some(Hl::SelfKw));
        assert_eq!(
            Lang::from_path(std::path::Path::new("a/b.slint")),
            Lang::Slint
        );
    }

    #[test]
    fn highlights_rust() {
        let k = kinds(
            "fn main() {\n    let s: String = \"hi\\n\"; // note\n    println!(\"{}\", s);\n}\n",
        );
        let find = |t: &str| k.iter().find(|(s, _)| s == t).map(|(_, h)| *h);
        assert_eq!(find("fn"), Some(Hl::Keyword));
        assert_eq!(find("main"), Some(Hl::Function));
        assert_eq!(find("String"), Some(Hl::Type));
        assert_eq!(find("// note"), Some(Hl::Comment));
        assert_eq!(find("println!"), Some(Hl::Macro).or(find("println")));
        assert!(
            k.iter().any(|(s, h)| s == "\\n" && *h == Hl::Escape),
            "{k:?}"
        );
    }

    #[test]
    fn incremental_update_matches_full_parse() {
        let mut d = Document::new("fn a() {}\n");
        let mut s = Syntax::new(Lang::Rust, d.rope()).unwrap();
        let sel = Selections::single(Selection::cursor(0));
        let edits = d.apply(
            vec![Change::insert(10, "struct B;\n")],
            &sel,
            sel.clone(),
            EditKind::Other,
        );
        s.update(&edits, d.rope());
        let fresh = Syntax::new(Lang::Rust, d.rope()).unwrap();
        let all = 0..d.rope().len_bytes();
        assert_eq!(
            s.highlights(d.rope(), all.clone()),
            fresh.highlights(d.rope(), all)
        );
    }

    #[test]
    fn knows_strings_and_comments() {
        let src = "let a = \"x\"; // c\n";
        let s = Syntax::new(Lang::Rust, &Rope::from_str(src)).unwrap();
        assert!(s.is_in_comment_or_string(10));
        assert!(s.is_in_comment_or_string(17));
        assert!(!s.is_in_comment_or_string(5));
    }
}
