//! JavaScript highlighting: the text cut into tokens by kind, each with its row and
//! column, so the window can draw them over the editor in a monospaced font.
//!
//! A tokenizer, not a parser: keywords, identifiers (function names, built-ins, `this`),
//! numbers, strings and template literals (with `${…}` inside), regular expressions,
//! comments, operators and punctuation. It carries on through mistakes, as it must while
//! someone is typing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Literal,
    Builtin,
    Function,
    Ident,
    Property,
    Number,
    String,
    Regex,
    Comment,
    Operator,
    Punct,
}

impl Kind {
    /// Its colour (a One Dark–style palette), as RGB.
    pub fn color(self) -> [u8; 3] {
        match self {
            Kind::Keyword => [0xc6, 0x78, 0xdd],
            Kind::Literal => [0xd1, 0x9a, 0x66],
            Kind::Builtin => [0xe5, 0xc0, 0x7b],
            Kind::Function => [0x61, 0xaf, 0xef],
            Kind::Ident => [0xe0, 0x6c, 0x75],
            Kind::Property => [0xab, 0xb2, 0xbf],
            Kind::Number => [0xd1, 0x9a, 0x66],
            Kind::String => [0x98, 0xc3, 0x79],
            Kind::Regex => [0x56, 0xb6, 0xc2],
            Kind::Comment => [0x7f, 0x84, 0x8e],
            Kind::Operator => [0x56, 0xb6, 0xc2],
            Kind::Punct => [0xab, 0xb2, 0xbf],
        }
    }
}

/// A token: where it starts (0-based row and column, in characters) and its text, which
/// never spans lines (a multi-line comment or string comes as one token per line).
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub row: usize,
    pub col: usize,
    pub text: String,
    pub kind: Kind,
}

const KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "of",
    "return",
    "set",
    "static",
    "super",
    "switch",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const LITERALS: &[&str] = &[
    "true",
    "false",
    "null",
    "undefined",
    "NaN",
    "Infinity",
    "this",
];
const BUILTINS: &[&str] = &[
    "Array",
    "BigInt",
    "Boolean",
    "console",
    "Date",
    "Error",
    "JSON",
    "Map",
    "Math",
    "Number",
    "Object",
    "Promise",
    "Proxy",
    "Reflect",
    "RegExp",
    "Set",
    "String",
    "Symbol",
    "WeakMap",
    "WeakSet",
    "globalThis",
    "parseFloat",
    "parseInt",
];

struct Lexer<'a> {
    chars: Vec<char>,
    i: usize,
    row: usize,
    col: usize,
    out: Vec<Token>,
    /// Whether a `/` here starts a regex (after an operator or `(`) or divides.
    regex_ok: bool,
    /// Open `${` in template literals: the brace depth each one closes at.
    templates: Vec<usize>,
    depth: usize,
    _src: &'a str,
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).copied()
    }

    /// Emits `chars[start..self.i]` as `kind`, split at line breaks.
    fn emit(&mut self, start: usize, start_row: usize, start_col: usize, kind: Kind) {
        let (mut row, mut col) = (start_row, start_col);
        let mut piece = String::new();
        let mut piece_col = col;
        for &c in &self.chars[start..self.i] {
            if c == '\n' {
                if !piece.is_empty() {
                    self.out.push(Token {
                        row,
                        col: piece_col,
                        text: std::mem::take(&mut piece),
                        kind,
                    });
                }
                row += 1;
                col = 0;
                piece_col = 0;
            } else {
                if piece.is_empty() {
                    piece_col = col;
                }
                piece.push(c);
                col += 1;
            }
        }
        if !piece.is_empty() {
            self.out.push(Token {
                row,
                col: piece_col,
                text: piece,
                kind,
            });
        }
    }

    /// Moves over one character, keeping row and column.
    fn bump(&mut self) {
        if self.peek(0) == Some('\n') {
            self.row += 1;
            self.col = 0;
        } else {
            self.col += 1;
        }
        self.i += 1;
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            let (start, row, col) = (self.i, self.row, self.col);
            if c.is_whitespace() {
                self.bump();
                continue;
            }
            // Comments.
            if c == '/' && self.peek(1) == Some('/') {
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.bump();
                }
                self.emit(start, row, col, Kind::Comment);
                continue;
            }
            if c == '/' && self.peek(1) == Some('*') {
                self.bump();
                self.bump();
                while let Some(c) = self.peek(0) {
                    if c == '*' && self.peek(1) == Some('/') {
                        self.bump();
                        self.bump();
                        break;
                    }
                    self.bump();
                }
                self.emit(start, row, col, Kind::Comment);
                continue;
            }
            // Strings.
            if c == '"' || c == '\'' {
                self.bump();
                while let Some(d) = self.peek(0) {
                    if d == '\\' {
                        self.bump();
                        self.bump();
                        continue;
                    }
                    if d == '\n' {
                        break;
                    }
                    self.bump();
                    if d == c {
                        break;
                    }
                }
                self.emit(start, row, col, Kind::String);
                self.regex_ok = false;
                continue;
            }
            // Template literals, and coming back into one after `${…}`.
            if c == '`' || (c == '}' && self.templates.last() == Some(&self.depth)) {
                if c == '}' {
                    self.templates.pop();
                    self.depth -= 1;
                }
                self.template_part(start, row, col);
                continue;
            }
            // Numbers.
            if c.is_ascii_digit() || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit()))
            {
                while self
                    .peek(0)
                    .is_some_and(|d| d.is_ascii_alphanumeric() || d == '.' || d == '_')
                {
                    self.bump();
                }
                self.emit(start, row, col, Kind::Number);
                self.regex_ok = false;
                continue;
            }
            // Words.
            if c.is_alphabetic() || c == '_' || c == '$' {
                while self
                    .peek(0)
                    .is_some_and(|d| d.is_alphanumeric() || d == '_' || d == '$')
                {
                    self.bump();
                }
                let word: String = self.chars[start..self.i].iter().collect();
                let after_dot = self
                    .out
                    .last()
                    .is_some_and(|t| t.text == "." || t.text == "?.");
                let next = self.chars[self.i..]
                    .iter()
                    .find(|c| !c.is_whitespace())
                    .copied();
                let kind = if after_dot {
                    if next == Some('(') {
                        Kind::Function
                    } else {
                        Kind::Property
                    }
                } else if KEYWORDS.contains(&word.as_str()) {
                    Kind::Keyword
                } else if LITERALS.contains(&word.as_str()) {
                    Kind::Literal
                } else if BUILTINS.contains(&word.as_str()) {
                    Kind::Builtin
                } else if next == Some('(') || self.out.last().is_some_and(|t| t.text == "function")
                {
                    Kind::Function
                } else if word.chars().next().is_some_and(char::is_uppercase) {
                    Kind::Builtin
                } else {
                    Kind::Ident
                };
                // After a keyword like `return`, a `/` starts a regex.
                self.regex_ok = kind == Kind::Keyword;
                self.emit(start, row, col, kind);
                continue;
            }
            // Regular expressions.
            if c == '/' && self.regex_ok {
                self.bump();
                let mut class = false;
                while let Some(d) = self.peek(0) {
                    if d == '\n' {
                        break;
                    }
                    self.bump();
                    match d {
                        '\\' => self.bump(),
                        '[' => class = true,
                        ']' => class = false,
                        '/' if !class => break,
                        _ => {}
                    }
                }
                while self.peek(0).is_some_and(char::is_alphabetic) {
                    self.bump();
                }
                self.emit(start, row, col, Kind::Regex);
                self.regex_ok = false;
                continue;
            }
            // Operators and punctuation.
            match c {
                '{' => self.depth += 1,
                '}' => self.depth = self.depth.saturating_sub(1),
                _ => {}
            }
            self.bump();
            if "=<>!&|+-*/%^?:~".contains(c) {
                while self.peek(0).is_some_and(|d| "=<>&|+-*/%^?.".contains(d))
                    && self.i - start < 4
                {
                    self.bump();
                }
                self.emit(start, row, col, Kind::Operator);
                self.regex_ok = true;
            } else {
                // `?.` and `...` as one.
                if c == '.' && self.peek(0) == Some('.') && self.peek(1) == Some('.') {
                    self.bump();
                    self.bump();
                }
                self.emit(start, row, col, Kind::Punct);
                self.regex_ok = matches!(c, '(' | '[' | ',' | ';' | '{' | '}');
            }
        }
    }

    /// From a `` ` `` (or the `}` ending a `${…}`): up to the closing `` ` `` or the next `${`.
    fn template_part(&mut self, start: usize, row: usize, col: usize) {
        self.bump();
        while let Some(d) = self.peek(0) {
            if d == '\\' {
                self.bump();
                self.bump();
                continue;
            }
            if d == '`' {
                self.bump();
                break;
            }
            if d == '$' && self.peek(1) == Some('{') {
                self.bump();
                self.bump();
                self.emit(start, row, col, Kind::String);
                self.depth += 1;
                self.templates.push(self.depth);
                return;
            }
            self.bump();
        }
        self.emit(start, row, col, Kind::String);
        self.regex_ok = false;
    }
}

pub fn tokens(src: &str) -> Vec<Token> {
    let mut l = Lexer {
        chars: src.chars().collect(),
        i: 0,
        row: 0,
        col: 0,
        out: Vec::new(),
        regex_ok: true,
        templates: Vec::new(),
        depth: 0,
        _src: src,
    };
    l.run();
    l.out
}

/// Tokens on rows `first..last`, neighbours of the same kind on a line joined (fewer
/// texts to draw).
pub fn visible(tokens: &[Token], first: usize, last: usize) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    for t in tokens.iter().filter(|t| t.row >= first && t.row < last) {
        if let Some(prev) = out.last_mut()
            && prev.row == t.row
            && prev.kind == t.kind
            && prev.col + prev.text.chars().count() == t.col
        {
            prev.text.push_str(&t.text);
            continue;
        }
        out.push(t.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<(String, Kind)> {
        tokens(src).into_iter().map(|t| (t.text, t.kind)).collect()
    }

    #[test]
    fn words_numbers_strings() {
        let k = kinds("const x = 42; return \"hi\" + 'there';");
        assert_eq!(k[0], ("const".into(), Kind::Keyword));
        assert_eq!(k[1], ("x".into(), Kind::Ident));
        assert_eq!(k[2], ("=".into(), Kind::Operator));
        assert_eq!(k[3], ("42".into(), Kind::Number));
        assert!(k.contains(&("\"hi\"".into(), Kind::String)));
        assert!(k.contains(&("'there'".into(), Kind::String)));
    }

    #[test]
    fn functions_properties_builtins() {
        let k = kinds("function fib(n) { console.log(Math.max(n, list.length)); }");
        assert_eq!(k[1], ("fib".into(), Kind::Function));
        assert!(k.contains(&("console".into(), Kind::Builtin)));
        assert!(k.contains(&("log".into(), Kind::Function)));
        assert!(k.contains(&("length".into(), Kind::Property)));
    }

    #[test]
    fn templates_with_expressions() {
        let k = kinds("`a ${b + 1} c`");
        assert_eq!(k[0], ("`a ${".into(), Kind::String));
        assert_eq!(k[1], ("b".into(), Kind::Ident));
        assert_eq!(k[3], ("1".into(), Kind::Number));
        assert_eq!(k[4], ("} c`".into(), Kind::String));
    }

    #[test]
    fn comments_and_regexes_span_correctly() {
        let t = tokens("a /* one\ntwo */ b\nlet r = /ab+c/gi; x = y / 2;");
        assert_eq!(t[1].text, "/* one");
        assert_eq!((t[2].row, t[2].col, t[2].text.as_str()), (1, 0, "two */"));
        let r = t.iter().find(|t| t.kind == Kind::Regex).unwrap();
        assert_eq!(r.text, "/ab+c/gi");
        // After a name, `/` divides.
        assert_eq!(t.iter().filter(|t| t.kind == Kind::Regex).count(), 1);
        let line = tokens("// note\nx");
        assert_eq!(line[0].kind, Kind::Comment);
        assert_eq!((line[1].row, line[1].col), (1, 0));
    }

    #[test]
    fn only_visible_rows_merged() {
        let t = tokens("a\n(b)\nc");
        let v = visible(&t, 1, 2);
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|t| t.row == 1));
        let merged = visible(&tokens("((("), 0, 1);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].text, "(((");
    }
}
