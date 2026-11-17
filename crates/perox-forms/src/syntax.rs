//! `.pxui` syntax: tokens and a parser that records exact byte spans for everything,
//! so edits can change one value and leave every other byte (comments, blank lines,
//! hand formatting) untouched.
//!
//! ```text
//! file      := element*
//! element   := Type [id] '{' member* '}'
//! member    := property | handler | element
//! property  := name ':' value            (ends at a newline, ';' or '}')
//! handler   := 'on' event ':' method
//! value     := string | number | number 'x' number | '#rrggbb[aa]' | true | false
//!            | identifier | '[' value (',' value)* ']'
//! ```
//! Comments: `// …` and `/* … */`.

use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Bool(bool),
    /// A bare word: an enum-like choice (`align: center`) or a reference.
    Ident(String),
    /// `640 x 480`
    Size(f64, f64),
    /// RGBA
    Color([u8; 4]),
    List(Vec<Value>),
}

impl Value {
    /// The value as `.pxui` source text.
    pub fn to_source(&self) -> String {
        match self {
            Value::Str(s) => {
                let mut out = String::from('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        c => out.push(c),
                    }
                }
                out.push('"');
                out
            }
            Value::Num(n) => num(*n),
            Value::Bool(b) => b.to_string(),
            Value::Ident(i) => i.clone(),
            Value::Size(w, h) => format!("{} x {}", num(*w), num(*h)),
            Value::Color([r, g, b, a]) => {
                if *a == 255 {
                    format!("#{r:02x}{g:02x}{b:02x}")
                } else {
                    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
                }
            }
            Value::List(items) => {
                format!(
                    "[{}]",
                    items
                        .iter()
                        .map(Value::to_source)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Ident(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

fn num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: Value,
    /// From the name to the end of the value.
    pub span: Range<usize>,
    pub value_span: Range<usize>,
}

/// `on clicked: save_file`: an event wired to a method in the code-behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub event: String,
    pub method: String,
    pub span: Range<usize>,
    pub method_span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// The widget type: `Window`, `Button`, `VBox`…
    pub kind: String,
    /// Optional name; named widgets become fields in the generated code.
    pub id: Option<String>,
    /// From the type name to the closing brace.
    pub span: Range<usize>,
    pub kind_span: Range<usize>,
    pub id_span: Option<Range<usize>>,
    /// Byte offset of `{` and of `}`.
    pub open: usize,
    pub close: usize,
    pub props: Vec<Property>,
    pub handlers: Vec<Handler>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn prop(&self, name: &str) -> Option<&Value> {
        self.props.iter().find(|p| p.name == name).map(|p| &p.value)
    }

    pub fn str_prop(&self, name: &str) -> Option<&str> {
        self.prop(name).and_then(Value::as_str)
    }

    pub fn num_prop(&self, name: &str) -> Option<f64> {
        self.prop(name).and_then(Value::as_f64)
    }

    /// Depth-first, with each element's index path from the root.
    pub fn walk<'a>(&'a self, path: &mut Vec<usize>, f: &mut dyn FnMut(&'a Element, &[usize])) {
        f(self, path);
        for (i, c) in self.children.iter().enumerate() {
            path.push(i);
            c.walk(path, f);
            path.pop();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub span: Range<usize>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub elements: Vec<Element>,
    pub errors: Vec<ParseError>,
}

impl File {
    /// The form's root element (the first top-level one).
    pub fn root(&self) -> Option<&Element> {
        self.elements.first()
    }

    /// The element at an index path from the root (`[]` is the root).
    pub fn at(&self, path: &[usize]) -> Option<&Element> {
        let mut e = self.root()?;
        for i in path {
            e = e.children.get(*i)?;
        }
        Some(e)
    }

    /// The path of the element named `id`.
    pub fn find(&self, id: &str) -> Option<Vec<usize>> {
        let root = self.root()?;
        let mut found = None;
        root.walk(&mut Vec::new(), &mut |e, p| {
            if found.is_none() && e.id.as_deref() == Some(id) {
                found = Some(p.to_vec());
            }
        });
        found
    }

    /// The innermost element whose source span contains `offset`.
    pub fn element_at_offset(&self, offset: usize) -> Option<Vec<usize>> {
        let root = self.root()?;
        let mut best = None;
        root.walk(&mut Vec::new(), &mut |e, p| {
            if e.span.contains(&offset) {
                best = Some(p.to_vec());
            }
        });
        best
    }
}

// ── lexer ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(f64),
    Color([u8; 4]),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Semi,
    Comma,
    Newline,
    Bad(char),
}

fn lex(src: &str) -> (Vec<(Tok, Range<usize>)>, Vec<ParseError>) {
    let bytes = src.as_bytes();
    let mut toks = Vec::new();
    let mut errors = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let start = i;
        match c {
            b'\n' => {
                toks.push((Tok::Newline, i..i + 1));
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    errors.push(ParseError {
                        span: start..bytes.len(),
                        message: "unclosed comment".into(),
                    });
                }
                i = (i + 2).min(bytes.len());
            }
            b'{' => {
                toks.push((Tok::LBrace, i..i + 1));
                i += 1;
            }
            b'}' => {
                toks.push((Tok::RBrace, i..i + 1));
                i += 1;
            }
            b'[' => {
                toks.push((Tok::LBracket, i..i + 1));
                i += 1;
            }
            b']' => {
                toks.push((Tok::RBracket, i..i + 1));
                i += 1;
            }
            b':' => {
                toks.push((Tok::Colon, i..i + 1));
                i += 1;
            }
            b';' => {
                toks.push((Tok::Semi, i..i + 1));
                i += 1;
            }
            b',' => {
                toks.push((Tok::Comma, i..i + 1));
                i += 1;
            }
            b'"' => {
                i += 1;
                let mut s = String::new();
                let mut closed = false;
                while i < bytes.len() {
                    let ch = src[i..].chars().next().unwrap_or('"');
                    match ch {
                        '"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        '\\' => {
                            let next = src[i + 1..].chars().next().unwrap_or('\\');
                            s.push(match next {
                                'n' => '\n',
                                't' => '\t',
                                other => other,
                            });
                            i += 1 + next.len_utf8();
                        }
                        '\n' => break,
                        other => {
                            s.push(other);
                            i += other.len_utf8();
                        }
                    }
                }
                if !closed {
                    errors.push(ParseError {
                        span: start..i,
                        message: "unclosed string".into(),
                    });
                }
                toks.push((Tok::Str(s), start..i));
            }
            b'#' => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
                let hex = &src[start + 1..i];
                let byte = |k: usize| u8::from_str_radix(&hex[k..k + 2], 16).unwrap_or(0);
                match hex.len() {
                    6 => toks.push((Tok::Color([byte(0), byte(2), byte(4), 255]), start..i)),
                    8 => toks.push((Tok::Color([byte(0), byte(2), byte(4), byte(6)]), start..i)),
                    _ => {
                        errors.push(ParseError {
                            span: start..i,
                            message: "colours are #rrggbb or #rrggbbaa".into(),
                        });
                        toks.push((Tok::Color([0, 0, 0, 255]), start..i));
                    }
                }
            }
            b'0'..=b'9' | b'-' | b'.' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                match src[start..i].parse::<f64>() {
                    Ok(n) => toks.push((Tok::Num(n), start..i)),
                    Err(_) => {
                        errors.push(ParseError {
                            span: start..i,
                            message: "not a number".into(),
                        });
                        toks.push((Tok::Bad(c as char), start..i));
                    }
                }
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                toks.push((Tok::Ident(src[start..i].to_owned()), start..i));
            }
            _ => {
                let ch = src[i..].chars().next().unwrap_or('?');
                i += ch.len_utf8();
                toks.push((Tok::Bad(ch), start..i));
            }
        }
    }
    (toks, errors)
}

// ── parser ─────────────────────────────────────────────────────────────────

struct Parser {
    toks: Vec<(Tok, Range<usize>)>,
    pos: usize,
    errors: Vec<ParseError>,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn peek_at(&self, k: usize) -> Option<&Tok> {
        self.toks.get(self.pos + k).map(|(t, _)| t)
    }

    fn span(&self) -> Range<usize> {
        self.toks
            .get(self.pos)
            .map_or(self.end..self.end, |(_, s)| s.clone())
    }

    fn bump(&mut self) -> Option<(Tok, Range<usize>)> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Some(Tok::Newline | Tok::Semi)) {
            self.pos += 1;
        }
    }

    fn error(&mut self, span: Range<usize>, message: impl Into<String>) {
        self.errors.push(ParseError {
            span,
            message: message.into(),
        });
    }

    /// Skips to the end of the line (or a `}` that closes the current element).
    fn recover(&mut self) {
        while let Some(t) = self.peek() {
            if matches!(t, Tok::Newline | Tok::RBrace) {
                break;
            }
            self.pos += 1;
        }
    }

    fn element(&mut self) -> Option<Element> {
        let (Tok::Ident(kind), kind_span) = self.bump()? else {
            return None;
        };
        let (id, id_span) = match self.peek() {
            Some(Tok::Ident(id)) => {
                let id = id.clone();
                let s = self.span();
                self.pos += 1;
                (Some(id), Some(s))
            }
            _ => (None, None),
        };
        if self.peek() != Some(&Tok::LBrace) {
            let s = self.span();
            self.error(s, format!("expected `{{` after `{kind}`"));
            self.recover();
            return None;
        }
        let open = self.span().start;
        self.pos += 1;
        let mut el = Element {
            kind,
            id,
            span: kind_span.start..kind_span.end,
            kind_span,
            id_span,
            open,
            close: open,
            props: Vec::new(),
            handlers: Vec::new(),
            children: Vec::new(),
        };
        loop {
            self.skip_newlines();
            match self.peek() {
                None => {
                    self.error(
                        el.kind_span.clone(),
                        format!("`{}` is missing its closing `}}`", el.kind),
                    );
                    el.close = self.end;
                    el.span.end = self.end;
                    return Some(el);
                }
                Some(Tok::RBrace) => {
                    let s = self.span();
                    self.pos += 1;
                    el.close = s.start;
                    el.span.end = s.end;
                    return Some(el);
                }
                Some(Tok::Ident(word)) => {
                    let word = word.clone();
                    match (self.peek_at(1), self.peek_at(2)) {
                        (Some(Tok::Colon), _) => self.property(&mut el),
                        (Some(Tok::Ident(_)), Some(Tok::Colon)) if word == "on" => {
                            self.handler(&mut el)
                        }
                        (Some(Tok::LBrace), _) | (Some(Tok::Ident(_)), Some(Tok::LBrace)) => {
                            if let Some(child) = self.element() {
                                el.children.push(child);
                            }
                        }
                        _ => {
                            let s = self.span();
                            self.error(
                                s,
                                format!("expected `{word}: value`, `on event: method` or a widget"),
                            );
                            self.pos += 1;
                            self.recover();
                        }
                    }
                }
                Some(_) => {
                    let s = self.span();
                    self.error(s, "unexpected symbol");
                    self.pos += 1;
                    self.recover();
                }
            }
        }
    }

    fn property(&mut self, el: &mut Element) {
        let Some((Tok::Ident(name), name_span)) = self.bump() else {
            return;
        };
        self.pos += 1; // ':'
        let start = self.span().start;
        match self.value() {
            Some((value, value_span)) => {
                if el.props.iter().any(|p| p.name == name) {
                    self.error(name_span.clone(), format!("`{name}` is set twice"));
                }
                el.props.push(Property {
                    name,
                    value,
                    span: name_span.start..value_span.end,
                    value_span,
                });
            }
            None => {
                self.error(start..start, format!("`{name}` needs a value"));
                self.recover();
            }
        }
        if !matches!(
            self.peek(),
            None | Some(Tok::Newline | Tok::Semi | Tok::RBrace)
        ) {
            let s = self.span();
            self.error(s, "one property per line (or separate with `;`)");
            self.recover();
        }
    }

    fn handler(&mut self, el: &mut Element) {
        let on = self.span();
        self.pos += 1; // `on`
        let Some((Tok::Ident(event), _)) = self.bump() else {
            return;
        };
        self.pos += 1; // ':'
        match self.bump() {
            Some((Tok::Ident(method), method_span)) => el.handlers.push(Handler {
                event,
                method,
                span: on.start..method_span.end,
                method_span,
            }),
            other => {
                let s = other.map_or(on.clone(), |(_, s)| s);
                self.error(s, format!("`on {event}:` needs a method name"));
                self.recover();
            }
        }
    }

    fn value(&mut self) -> Option<(Value, Range<usize>)> {
        let (tok, span) = self.bump()?;
        Some(match tok {
            Tok::Str(s) => (Value::Str(s), span),
            Tok::Color(c) => (Value::Color(c), span),
            Tok::Num(w) => {
                // `640 x 480`
                if matches!(self.peek(), Some(Tok::Ident(x)) if x == "x")
                    && let Some(Tok::Num(h)) = self.peek_at(1).cloned()
                {
                    self.pos += 1;
                    let (_, hs) = self.bump()?;
                    (Value::Size(w, h), span.start..hs.end)
                } else {
                    (Value::Num(w), span)
                }
            }
            Tok::Ident(i) if i == "true" => (Value::Bool(true), span),
            Tok::Ident(i) if i == "false" => (Value::Bool(false), span),
            Tok::Ident(i) => (Value::Ident(i), span),
            Tok::LBracket => {
                let mut items = Vec::new();
                loop {
                    while matches!(self.peek(), Some(Tok::Newline)) {
                        self.pos += 1;
                    }
                    if let Some(Tok::RBracket) = self.peek() {
                        let end = self.span().end;
                        self.pos += 1;
                        return Some((Value::List(items), span.start..end));
                    }
                    let (v, _) = self.value()?;
                    items.push(v);
                    while matches!(self.peek(), Some(Tok::Newline)) {
                        self.pos += 1;
                    }
                    if let Some(Tok::Comma) = self.peek() {
                        self.pos += 1;
                    }
                }
            }
            _ => {
                self.pos -= 1;
                return None;
            }
        })
    }
}

/// Parses `.pxui` source. Always returns what it could understand plus the errors.
pub fn parse(src: &str) -> File {
    let (toks, mut errors) = lex(src);
    for (t, s) in &toks {
        if let Tok::Bad(c) = t {
            errors.push(ParseError {
                span: s.clone(),
                message: format!("unexpected `{c}`"),
            });
        }
    }
    let toks: Vec<_> = toks
        .into_iter()
        .filter(|(t, _)| !matches!(t, Tok::Bad(_)))
        .collect();
    let mut p = Parser {
        toks,
        pos: 0,
        errors: Vec::new(),
        end: src.len(),
    };
    let mut elements = Vec::new();
    loop {
        p.skip_newlines();
        match p.peek() {
            None => break,
            Some(Tok::Ident(_)) => {
                if let Some(e) = p.element() {
                    elements.push(e);
                }
            }
            Some(_) => {
                let s = p.span();
                p.error(s, "expected a widget, like `Window main { … }`");
                p.pos += 1;
            }
        }
    }
    errors.extend(p.errors);
    errors.sort_by_key(|e| e.span.start);
    File { elements, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"// The main window.
Window main_window {
    title: "Word \"Counter\""   // shown in the title bar
    size: 640 x 480
    resizable: true

    VBox {
        spacing: 8; margin: 12
        Button count {
            text: "Count"
            on clicked: count_clicked
        }
        Label result { text: "" color: #ff8800 }
        ComboBox mode { items: ["Words", "Lines"] }
    }
}
"#;

    #[test]
    fn parses_a_form_with_spans() {
        let f = parse(SAMPLE);
        let root = f.root().unwrap();
        assert_eq!(
            (root.kind.as_str(), root.id.as_deref()),
            ("Window", Some("main_window"))
        );
        assert_eq!(root.str_prop("title"), Some("Word \"Counter\""));
        assert_eq!(root.prop("size"), Some(&Value::Size(640.0, 480.0)));
        assert_eq!(root.prop("resizable"), Some(&Value::Bool(true)));
        let vbox = &root.children[0];
        assert_eq!(vbox.num_prop("spacing"), Some(8.0));
        assert_eq!(vbox.num_prop("margin"), Some(12.0));
        let button = &vbox.children[0];
        assert_eq!(button.handlers[0].event, "clicked");
        assert_eq!(button.handlers[0].method, "count_clicked");
        assert_eq!(
            &SAMPLE[button.handlers[0].method_span.clone()],
            "count_clicked"
        );
        let size = root.props.iter().find(|p| p.name == "size").unwrap();
        assert_eq!(&SAMPLE[size.value_span.clone()], "640 x 480");
        assert_eq!(f.find("mode"), Some(vec![0, 2]));
        assert_eq!(
            f.at(&[0, 2]).unwrap().prop("items"),
            Some(&Value::List(vec![
                Value::Str("Words".into()),
                Value::Str("Lines".into())
            ]))
        );
        assert_eq!(
            &SAMPLE[root.span.clone()],
            SAMPLE
                .trim()
                .trim_start_matches("// The main window.")
                .trim()
        );
    }

    #[test]
    fn this_line_has_two_properties_and_is_an_error() {
        // The sample's `Label result { text: "" color: … }` puts two properties on one line.
        let f = parse(SAMPLE);
        assert_eq!(f.errors.len(), 1, "{:?}", f.errors);
        assert!(f.errors[0].message.contains("one property per line"));
    }

    #[test]
    fn values_round_trip_through_source() {
        for v in [
            Value::Str("a \"b\"\nc".into()),
            Value::Num(12.0),
            Value::Num(0.5),
            Value::Size(640.0, 480.0),
            Value::Color([255, 136, 0, 255]),
            Value::Color([1, 2, 3, 4]),
            Value::Bool(false),
            Value::Ident("center".into()),
            Value::List(vec![Value::Num(1.0), Value::Str("x".into())]),
        ] {
            let src = format!("W {{ p: {} }}", v.to_source());
            let f = parse(&src);
            assert!(f.errors.is_empty(), "{src}: {:?}", f.errors);
            assert_eq!(f.root().unwrap().prop("p"), Some(&v), "{src}");
        }
    }

    #[test]
    fn recovers_from_errors_and_keeps_going() {
        let f =
            parse("Window w {\n  title \"x\"\n  Button b { text: }\n  Label l { text: \"ok\" }\n}");
        assert_eq!(f.errors.len(), 2, "{:?}", f.errors);
        let root = f.root().unwrap();
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[1].str_prop("text"), Some("ok"));
    }

    #[test]
    fn unclosed_things_are_reported() {
        assert!(!parse("Window w {").errors.is_empty());
        assert!(!parse("Window w { t: \"x }").errors.is_empty());
        assert!(!parse("Window w { c: #12 }").errors.is_empty());
    }

    #[test]
    fn finds_the_element_under_a_cursor() {
        let f = parse(SAMPLE);
        let at = SAMPLE.find("\"Count\"").unwrap();
        assert_eq!(f.element_at_offset(at), Some(vec![0, 0]));
    }
}
