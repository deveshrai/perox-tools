//! A `.slint` document as the designer sees it: imports, components, and each
//! component's element tree, with exact byte spans for everything. Built on Slint's own
//! lossless parser, so anything Slint accepts parses here too.
//!
//! Spans never include surrounding whitespace or comments. Element paths are child
//! indices from a component's root element (`[]` is the root); only plain child elements
//! count (`for` and `if` elements are kept in `others`, untouched).

use std::ops::Range;

use i_slint_compiler::diagnostics::{BuildDiagnostics, Spanned};
use i_slint_compiler::parser::{self, SyntaxKind, SyntaxNode};

/// A property value, as far as the designer understands it. Anything else is `Expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    /// A number with its unit (`12px` → `(12.0, "px")`, `0.5` → `(0.5, "")`).
    Num(f64, String),
    Bool(bool),
    /// `#rrggbb` / `#rrggbbaa`, as RGBA.
    Color([u8; 4]),
    /// A bare or dotted name: an enum value (`center`, `TextHorizontalAlignment.center`)
    /// or a reference.
    Ident(String),
    /// `["a", "b"]`
    List(Vec<Value>),
    /// Any other expression, as written.
    Expr(String),
}

impl Value {
    /// Reads an expression's source text.
    pub fn parse(src: &str) -> Value {
        let s = src.trim();
        if let Some(v) = string_literal(s) {
            return Value::Str(v);
        }
        match s {
            "true" => return Value::Bool(true),
            "false" => return Value::Bool(false),
            _ => {}
        }
        if let Some(c) = color_literal(s) {
            return Value::Color(c);
        }
        if let Some((n, unit)) = number_literal(s) {
            return Value::Num(n, unit);
        }
        if let Some(inner) = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            let items: Option<Vec<Value>> = split_top_level(inner)
                .into_iter()
                .filter(|i| !i.trim().is_empty())
                .map(|i| match Value::parse(i) {
                    Value::Expr(_) => None,
                    v => Some(v),
                })
                .collect();
            if let Some(items) = items {
                return Value::List(items);
            }
        }
        if !s.is_empty()
            && s.split('.').all(|part| {
                let mut c = part.chars();
                c.next()
                    .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
                    && c.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
        {
            return Value::Ident(s.to_owned());
        }
        Value::Expr(s.to_owned())
    }

    /// The value as `.slint` source text.
    pub fn to_source(&self) -> String {
        match self {
            Value::Str(s) => quote(s),
            Value::Num(n, unit) => format!("{}{unit}", num(*n)),
            Value::Bool(b) => b.to_string(),
            Value::Color([r, g, b, a]) => {
                if *a == 255 {
                    format!("#{r:02x}{g:02x}{b:02x}")
                } else {
                    format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
                }
            }
            Value::Ident(i) | Value::Expr(i) => i.clone(),
            Value::List(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(Value::to_source)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n, _) => Some(*n),
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

/// A string as a `.slint` string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::from('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        let s = format!("{n:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    }
}

fn string_literal(s: &str) -> Option<String> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return None, // two literals, or a template
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                _ => return None, // `\{…}` interpolation or `\u{…}`: keep as an expression
            },
            c => out.push(c),
        }
    }
    Some(out)
}

fn color_literal(s: &str) -> Option<[u8; 4]> {
    let hex = s.strip_prefix('#')?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    let nib = |i: usize| u8::from_str_radix(&hex[i..i + 1], 16).ok().map(|v| v * 17);
    match hex.len() {
        3 => Some([nib(0)?, nib(1)?, nib(2)?, 255]),
        4 => Some([nib(0)?, nib(1)?, nib(2)?, nib(3)?]),
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

fn number_literal(s: &str) -> Option<(f64, String)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, s),
    };
    let split = body
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(body.len());
    let (digits, unit) = body.split_at(split);
    if digits.is_empty() || !unit.chars().all(|c| c.is_ascii_alphabetic() || c == '%') {
        return None;
    }
    let n: f64 = digits.parse().ok()?;
    Some((if neg { -n } else { n }, unit.to_owned()))
}

/// Splits on commas that aren't inside brackets, parentheses or strings.
fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut in_str, mut escaped, mut start) = (0i32, false, false, 0);
    for (i, c) in s.char_indices() {
        if in_str {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// `name: value;`
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: Value,
    /// The whole binding, including its `;`.
    pub span: Range<usize>,
    /// The expression (without `;`).
    pub value_span: Range<usize>,
}

/// `event(args) => { body }`
#[derive(Debug, Clone, PartialEq)]
pub struct Handler {
    pub event: String,
    pub params: Vec<String>,
    /// The body's source, without the outer braces.
    pub body: String,
    pub span: Range<usize>,
    /// The `{ … }` block or expression.
    pub body_span: Range<usize>,
}

impl Handler {
    /// The root callback this handler forwards to, if its body is just
    /// `root.name(…);`.
    pub fn forwards_to(&self) -> Option<&str> {
        let b = self.body.trim().trim_end_matches(';').trim();
        let rest = b.strip_prefix("root.")?;
        let name = &rest[..rest.find('(')?];
        rest.ends_with(')').then_some(name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// `Button`, `VerticalBox`, `Rectangle`…
    pub kind: String,
    /// `ok` in `ok := Button { … }`.
    pub id: Option<String>,
    /// The whole element, including `id :=`.
    pub span: Range<usize>,
    pub kind_span: Range<usize>,
    pub id_span: Option<Range<usize>>,
    /// Offsets of `{` and `}`.
    pub open: usize,
    pub close: usize,
    pub props: Vec<Property>,
    pub handlers: Vec<Handler>,
    pub children: Vec<Element>,
    /// Every other member (declarations, `for`, `if`, states, animations…), in order.
    pub others: Vec<Range<usize>>,
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

    pub fn handler(&self, event: &str) -> Option<&Handler> {
        self.handlers.iter().find(|h| h.event == event)
    }

    /// Calls `f` for this element and every descendant, with its path.
    pub fn walk<'a>(&'a self, path: &mut Vec<usize>, f: &mut dyn FnMut(&'a Element, &[usize])) {
        f(self, path);
        for (i, c) in self.children.iter().enumerate() {
            path.push(i);
            c.walk(path, f);
            path.pop();
        }
    }

    pub fn at(&self, path: &[usize]) -> Option<&Element> {
        match path.split_first() {
            None => Some(self),
            Some((i, rest)) => self.children.get(*i)?.at(rest),
        }
    }

    /// Every member's span, in source order.
    pub fn member_spans(&self) -> Vec<Range<usize>> {
        let mut v: Vec<Range<usize>> = self
            .props
            .iter()
            .map(|p| p.span.clone())
            .chain(self.handlers.iter().map(|h| h.span.clone()))
            .chain(self.children.iter().map(|c| c.span.clone()))
            .chain(self.others.iter().cloned())
            .collect();
        v.sort_by_key(|r| r.start);
        v
    }
}

/// `callback name(type, …) -> type;` on a component.
#[derive(Debug, Clone, PartialEq)]
pub struct Callback {
    pub name: String,
    /// `(name, type)`; names are optional in Slint.
    pub params: Vec<(Option<String>, String)>,
    pub ret: Option<String>,
    pub span: Range<usize>,
}

/// `in-out property <type> name: value;`
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyDecl {
    pub name: String,
    pub ty: Option<String>,
    /// `in`, `out`, `in-out`, `private`, or empty.
    pub visibility: String,
    /// `x` in `property <t> p <=> x;`
    pub alias: Option<String>,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub name: String,
    pub exported: bool,
    /// The whole `component … { … }` (with `export` if present).
    pub span: Range<usize>,
    /// The root element; its `kind` is what the component inherits (`Window`), or empty.
    pub root: Element,
    pub callbacks: Vec<Callback>,
    pub properties: Vec<PropertyDecl>,
}

impl Component {
    pub fn at(&self, path: &[usize]) -> Option<&Element> {
        self.root.at(path)
    }

    /// The path of the element with this id.
    pub fn find(&self, id: &str) -> Option<Vec<usize>> {
        let mut found = None;
        self.root.walk(&mut Vec::new(), &mut |el, path| {
            if found.is_none() && el.id.as_deref() == Some(id) {
                found = Some(path.to_vec());
            }
        });
        found
    }

    /// The innermost element whose source contains `offset`.
    pub fn element_at_offset(&self, offset: usize) -> Option<Vec<usize>> {
        let mut best: Option<(usize, Vec<usize>)> = None;
        self.root.walk(&mut Vec::new(), &mut |el, path| {
            if el.span.contains(&offset)
                && best.as_ref().is_none_or(|(len, _)| el.span.len() <= *len)
            {
                best = Some((el.span.len(), path.to_vec()));
            }
        });
        best.map(|(_, p)| p)
    }

    /// The root property that exposes `target` (`name_edit.text`), if any.
    /// The root property that makes `target` (`id.prop`) usable from code: one aliasing it
    /// (`<=> id.prop`), or one it's bound to (`prop: root.name;`), for properties Slint
    /// only lets you bind.
    pub fn exposing(&self, target: &str) -> Option<&PropertyDecl> {
        let t = normalize(target);
        if let Some(p) = self
            .properties
            .iter()
            .find(|p| p.alias.as_deref().is_some_and(|a| normalize(a) == t))
        {
            return Some(p);
        }
        let (id, prop) = t.split_once('.')?;
        let path = self.find(id).or_else(|| {
            let mut found = None;
            self.root.walk(&mut Vec::new(), &mut |el, path| {
                if found.is_none() && el.id.as_deref().map(normalize).as_deref() == Some(id) {
                    found = Some(path.to_vec());
                }
            });
            found
        })?;
        let el = self.at(&path)?;
        let bound = el.props.iter().find(|p| normalize(&p.name) == prop)?;
        let source = bound.value.to_source();
        let name = source.strip_prefix("root.")?;
        self.property(name).filter(|p| p.alias.is_none())
    }

    pub fn property(&self, name: &str) -> Option<&PropertyDecl> {
        let n = normalize(name);
        self.properties.iter().find(|p| normalize(&p.name) == n)
    }

    pub fn callback(&self, name: &str) -> Option<&Callback> {
        let n = normalize(name);
        self.callbacks.iter().find(|c| normalize(&c.name) == n)
    }

    /// Ends with `Window` or `Dialog`: it opens as a window.
    pub fn is_window(&self) -> bool {
        matches!(self.root.kind.as_str(), "Window" | "Dialog" | "PopupWindow")
    }
}

/// `import { A, B as C } from "file.slint";`
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// Names as imported (`A`, `B`), not their local aliases.
    pub names: Vec<String>,
    pub from: String,
    pub span: Range<usize>,
    /// Offset of the list's `}`.
    pub list_close: Option<usize>,
}

/// `export { A, B as C } from "file.slint";` or `export * from "file.slint";`
#[derive(Debug, Clone, PartialEq)]
pub struct ReExport {
    /// `(name in that file, exported name)`; empty with `all`.
    pub names: Vec<(String, String)>,
    pub all: bool,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub span: Range<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document {
    pub imports: Vec<Import>,
    pub re_exports: Vec<ReExport>,
    pub components: Vec<Component>,
    pub errors: Vec<ParseError>,
}

impl Document {
    pub fn component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }

    pub fn component_index(&self, name: &str) -> Option<usize> {
        self.components.iter().position(|c| c.name == name)
    }

    /// The component the designer opens: the last exported window, else the last
    /// exported component, else the last one.
    pub fn main_component(&self) -> Option<usize> {
        let last = |f: &dyn Fn(&Component) -> bool| self.components.iter().rposition(f);
        last(&|c| c.exported && c.is_window())
            .or_else(|| last(&|c| c.exported))
            .or_else(|| self.components.len().checked_sub(1))
    }

    /// True if `name` is imported from `from`.
    pub fn imports_name(&self, name: &str, from: &str) -> bool {
        self.imports
            .iter()
            .any(|i| i.from == from && i.names.iter().any(|n| n == name))
    }
}

/// Slint treats `-` and `_` in names as the same.
pub fn normalize(name: &str) -> String {
    name.replace('-', "_")
}

/// The range of a node without leading/trailing whitespace and comments.
fn trimmed(node: &SyntaxNode) -> Range<usize> {
    let mut first = None;
    let mut last = None;
    for t in node.descendants_with_tokens() {
        if let rowan::NodeOrToken::Token(t) = t
            && !matches!(
                t.kind(),
                SyntaxKind::Whitespace | SyntaxKind::Comment | SyntaxKind::Eof
            )
        {
            let r = t.text_range();
            if first.is_none() {
                first = Some(usize::from(r.start()));
            }
            last = Some(usize::from(r.end()));
        }
    }
    let r = node.text_range();
    first.unwrap_or(r.start().into())..last.unwrap_or(r.start().into())
}

fn token_range(t: &parser::SyntaxToken) -> Range<usize> {
    let r = t.text_range();
    r.start().into()..r.end().into()
}

fn first_ident(node: &SyntaxNode) -> Option<String> {
    node.child_text(SyntaxKind::Identifier)
        .map(|s| s.to_string())
}

fn text_of(src: &str, r: &Range<usize>) -> String {
    src[r.clone()].to_owned()
}

fn build_element(
    src: &str,
    el: &SyntaxNode,
    outer: &SyntaxNode,
    id: Option<(String, Range<usize>)>,
) -> Element {
    let kind_node = el.child_node(SyntaxKind::QualifiedName);
    let kind_span = kind_node.as_ref().map(trimmed).unwrap_or(0..0);
    let kind = text_of(src, &kind_span);
    let open = el
        .child_token(SyntaxKind::LBrace)
        .map(|t| token_range(&t).start)
        .unwrap_or(kind_span.end);
    let close = el
        .child_token(SyntaxKind::RBrace)
        .map(|t| token_range(&t).start)
        .unwrap_or(open);
    let mut out = Element {
        kind,
        id: id.as_ref().map(|(n, _)| n.clone()),
        span: trimmed(outer),
        kind_span,
        id_span: id.map(|(_, r)| r),
        open,
        close,
        props: Vec::new(),
        handlers: Vec::new(),
        children: Vec::new(),
        others: Vec::new(),
    };
    for child in el.children() {
        match child.kind() {
            SyntaxKind::QualifiedName => {}
            SyntaxKind::Binding => {
                let name = first_ident(&child).unwrap_or_default();
                let expr = child.child_node(SyntaxKind::BindingExpression);
                let value_span = expr
                    .as_ref()
                    .and_then(|b| b.children().next())
                    .map(|n| trimmed(&n))
                    .unwrap_or(0..0);
                out.props.push(Property {
                    name,
                    value: Value::parse(&src[value_span.clone()]),
                    span: trimmed(&child),
                    value_span,
                });
            }
            SyntaxKind::CallbackConnection => {
                let event = first_ident(&child).unwrap_or_default();
                let params = child
                    .children()
                    .filter(|c| c.kind() == SyntaxKind::DeclaredIdentifier)
                    .map(|c| src[trimmed(&c)].to_owned())
                    .collect();
                let body_node = child
                    .child_node(SyntaxKind::CodeBlock)
                    .or_else(|| child.child_node(SyntaxKind::Expression));
                let body_span = body_node.as_ref().map(trimmed).unwrap_or(0..0);
                let body = match body_node.as_ref().map(|n| n.kind()) {
                    Some(SyntaxKind::CodeBlock) => {
                        let b = &src[body_span.clone()];
                        b.strip_prefix('{')
                            .and_then(|b| b.strip_suffix('}'))
                            .unwrap_or(b)
                            .trim()
                            .to_owned()
                    }
                    _ => src[body_span.clone()].to_owned(),
                };
                out.handlers.push(Handler {
                    event,
                    params,
                    body,
                    span: trimmed(&child),
                    body_span,
                });
            }
            SyntaxKind::SubElement => {
                if let Some(inner) = child.child_node(SyntaxKind::Element) {
                    let id = child
                        .child_token(SyntaxKind::Identifier)
                        .map(|t| (t.text().to_owned(), token_range(&t)));
                    out.children.push(build_element(src, &inner, &child, id));
                }
            }
            _ => out.others.push(trimmed(&child)),
        }
    }
    out
}

fn build_component(
    src: &str,
    comp: &SyntaxNode,
    exported: bool,
    outer: &SyntaxNode,
) -> Option<Component> {
    let name_node = comp.child_node(SyntaxKind::DeclaredIdentifier)?;
    let name = src[trimmed(&name_node)].to_owned();
    let el = comp.child_node(SyntaxKind::Element)?;
    let root = build_element(src, &el, &el, None);
    let mut callbacks = Vec::new();
    let mut properties = Vec::new();
    for c in el.children() {
        match c.kind() {
            SyntaxKind::CallbackDeclaration => {
                let Some(n) = c.child_node(SyntaxKind::DeclaredIdentifier) else {
                    continue;
                };
                let params = c
                    .children()
                    .filter(|p| p.kind() == SyntaxKind::CallbackDeclarationParameter)
                    .map(|p| {
                        let name = p
                            .child_node(SyntaxKind::DeclaredIdentifier)
                            .map(|d| src[trimmed(&d)].to_owned());
                        let ty = p
                            .child_node(SyntaxKind::Type)
                            .map(|t| src[trimmed(&t)].to_owned())
                            .unwrap_or_default();
                        (name, ty)
                    })
                    .collect();
                let ret = c
                    .child_node(SyntaxKind::ReturnType)
                    .and_then(|r| r.child_node(SyntaxKind::Type))
                    .map(|t| src[trimmed(&t)].to_owned());
                callbacks.push(Callback {
                    name: src[trimmed(&n)].to_owned(),
                    params,
                    ret,
                    span: trimmed(&c),
                });
            }
            SyntaxKind::PropertyDeclaration => {
                if let Some(n) = c.child_node(SyntaxKind::DeclaredIdentifier) {
                    let span = trimmed(&c);
                    let head = &src[span.start..trimmed(&n).start];
                    let visibility = head
                        .split_whitespace()
                        .find(|w| matches!(*w, "in" | "out" | "in-out" | "private"))
                        .unwrap_or_default()
                        .to_owned();
                    properties.push(PropertyDecl {
                        name: src[trimmed(&n)].to_owned(),
                        ty: c
                            .child_node(SyntaxKind::Type)
                            .map(|t| src[trimmed(&t)].to_owned()),
                        visibility,
                        alias: c
                            .child_node(SyntaxKind::TwoWayBinding)
                            .and_then(|t| t.child_node(SyntaxKind::Expression))
                            .map(|e| src[trimmed(&e)].to_owned()),
                        span,
                    });
                }
            }
            _ => {}
        }
    }
    Some(Component {
        name,
        exported,
        span: trimmed(outer),
        root,
        callbacks,
        properties,
    })
}

fn unquote(s: &str) -> String {
    string_literal(s).unwrap_or_else(|| s.trim_matches('"').to_owned())
}

/// Parses a `.slint` document. Never fails: syntax errors are in `errors`, and as much of
/// the tree as could be read is returned.
pub fn parse(src: &str) -> Document {
    let mut diag = BuildDiagnostics::default();
    let root = parser::parse(src.to_owned(), None, &mut diag);
    let mut doc = Document::default();
    for d in diag.iter() {
        if d.level() == i_slint_compiler::diagnostics::DiagnosticLevel::Error {
            let s = Spanned::span(d);
            let start = s.offset;
            doc.errors.push(ParseError {
                span: start..start + s.length,
                message: d.message().to_owned(),
            });
        }
    }
    for node in root.children() {
        match node.kind() {
            SyntaxKind::Component => {
                doc.components
                    .extend(build_component(src, &node, false, &node));
            }
            SyntaxKind::ExportsList => {
                if let Some(comp) = node.child_node(SyntaxKind::Component) {
                    doc.components
                        .extend(build_component(src, &comp, true, &node));
                } else if let Some(module) = node.child_node(SyntaxKind::ExportModule) {
                    let from = module
                        .child_token(SyntaxKind::StringLiteral)
                        .map(|t| unquote(t.text()))
                        .unwrap_or_default();
                    let names: Vec<(String, String)> = node
                        .children()
                        .filter(|c| c.kind() == SyntaxKind::ExportSpecifier)
                        .filter_map(|s| export_names(src, &s))
                        .collect();
                    doc.re_exports.push(ReExport {
                        all: names.is_empty(),
                        names,
                        from,
                    });
                } else {
                    // `export { A, B }` of local components: mark them exported.
                    let names: Vec<String> = node
                        .children()
                        .filter(|c| c.kind() == SyntaxKind::ExportSpecifier)
                        .filter_map(|s| export_names(src, &s).map(|(n, _)| n))
                        .collect();
                    for c in &mut doc.components {
                        if names.contains(&c.name) {
                            c.exported = true;
                        }
                    }
                }
            }
            SyntaxKind::ImportSpecifier => {
                let from = node
                    .child_token(SyntaxKind::StringLiteral)
                    .map(|t| unquote(t.text()))
                    .unwrap_or_default();
                let list = node.child_node(SyntaxKind::ImportIdentifierList);
                let names = list
                    .iter()
                    .flat_map(|l| l.children())
                    .filter(|c| c.kind() == SyntaxKind::ImportIdentifier)
                    .filter_map(|c| c.child_node(SyntaxKind::ExternalName))
                    .map(|n| src[trimmed(&n)].to_owned())
                    .collect();
                let list_close = list
                    .and_then(|l| l.child_token(SyntaxKind::RBrace))
                    .map(|t| token_range(&t).start);
                doc.imports.push(Import {
                    names,
                    from,
                    span: trimmed(&node),
                    list_close,
                });
            }
            _ => {}
        }
    }
    // `export { A }` may come before `component A`.
    let late: Vec<String> = root
        .children()
        .filter(|n| {
            n.kind() == SyntaxKind::ExportsList
                && n.child_node(SyntaxKind::Component).is_none()
                && n.child_node(SyntaxKind::ExportModule).is_none()
        })
        .flat_map(|n| n.children().collect::<Vec<_>>())
        .filter(|c| c.kind() == SyntaxKind::ExportSpecifier)
        .filter_map(|s| export_names(src, &s).map(|(n, _)| n))
        .collect();
    for c in &mut doc.components {
        if late.contains(&c.name) {
            c.exported = true;
        }
    }
    doc
}

/// `A` or `A as B` in an export list → `(A, B)`.
fn export_names(src: &str, spec: &SyntaxNode) -> Option<(String, String)> {
    let inner = spec.child_node(SyntaxKind::ExportIdentifier)?;
    let name = src[trimmed(&inner)].to_owned();
    let alias = spec
        .child_node(SyntaxKind::ExportName)
        .map(|n| src[trimmed(&n)].to_owned())
        .unwrap_or_else(|| name.clone());
    Some((name, alias))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const SRC: &str = r#"import { Button, LineEdit } from "std-widgets.slint";
// The main window.
export component MainWindow inherits Window {
    title: "Hello";
    callback ok_clicked();
    callback name_edited(text: string);
    in-out property <int> count: 3;
    VerticalLayout {
        name := LineEdit { placeholder-text: "Name"; edited(t) => { root.name_edited(t); } }
        ok := Button {
            text: "OK";
            clicked => { root.ok_clicked(); }
        }
        for i in 3: Text { text: i; }
    }
}
"#;

    #[test]
    fn reads_components_elements_and_spans() {
        let d = parse(SRC);
        assert!(d.errors.is_empty(), "{:?}", d.errors);
        assert_eq!(d.imports[0].names, ["Button", "LineEdit"]);
        assert_eq!(d.imports[0].from, "std-widgets.slint");
        let c = &d.components[0];
        assert!(c.exported && c.is_window());
        assert_eq!(c.name, "MainWindow");
        assert_eq!(c.root.kind, "Window");
        assert_eq!(c.root.str_prop("title"), Some("Hello"));
        assert_eq!(c.callbacks.len(), 2);
        assert_eq!(
            c.callbacks[1].params,
            [(Some("text".into()), "string".into())]
        );
        assert_eq!(c.properties[0].name, "count");
        assert_eq!(c.properties[0].visibility, "in-out");
        let d = parse(
            "export component A { in-out property <string> name <=> edit.text; edit := TextInput { } }",
        );
        let a = &d.components[0];
        assert_eq!(a.properties[0].alias.as_deref(), Some("edit.text"));
        assert_eq!(
            a.exposing("edit.text").map(|p| p.name.as_str()),
            Some("name")
        );
        assert!(a.property("name").is_some());
        let col = c.at(&[0]).unwrap();
        assert_eq!(col.kind, "VerticalLayout");
        assert_eq!(col.children.len(), 2, "the `for` isn't a child");
        assert_eq!(col.others.len(), 1);
        let ok = c.at(&[0, 1]).unwrap();
        assert_eq!(ok.id.as_deref(), Some("ok"));
        assert_eq!(&SRC[ok.span.clone()][..13], "ok := Button ");
        assert!(SRC[ok.span.clone()].ends_with('}'));
        assert_eq!(&SRC[ok.kind_span.clone()], "Button");
        assert_eq!(
            ok.handler("clicked").unwrap().forwards_to(),
            Some("ok_clicked")
        );
        let name = c.at(&[0, 0]).unwrap();
        assert_eq!(name.str_prop("placeholder-text"), Some("Name"));
        assert_eq!(name.handlers[0].params, ["t"]);
        assert_eq!(
            &SRC[name.props[0].span.clone()],
            "placeholder-text: \"Name\";"
        );
        assert_eq!(c.find("ok"), Some(vec![0, 1]));
        assert_eq!(
            c.element_at_offset(SRC.find("OK").unwrap()),
            Some(vec![0, 1])
        );
        assert_eq!(d.main_component(), Some(0));
    }

    #[test]
    fn values() {
        assert_eq!(Value::parse("\"a\\\"b\""), Value::Str("a\"b".into()));
        assert_eq!(Value::parse("12px"), Value::Num(12.0, "px".into()));
        assert_eq!(Value::parse("-0.5"), Value::Num(-0.5, "".into()));
        assert_eq!(Value::parse("50%"), Value::Num(50.0, "%".into()));
        assert_eq!(Value::parse("#ff000080"), Value::Color([255, 0, 0, 128]));
        assert_eq!(Value::parse("true"), Value::Bool(true));
        assert_eq!(
            Value::parse("TextHorizontalAlignment.center"),
            Value::Ident("TextHorizontalAlignment.center".into())
        );
        assert_eq!(
            Value::parse("[\"a\", \"b, c\"]"),
            Value::List(vec![Value::Str("a".into()), Value::Str("b, c".into())])
        );
        assert_eq!(Value::parse("a + b"), Value::Expr("a + b".into()));
        assert_eq!(Value::parse("\"x\\{y}\""), Value::Expr("\"x\\{y}\"".into()));
        for v in ["12px", "\"q\\\"\"", "#102030", "[\"a\", \"b\"]", "0.25"] {
            assert_eq!(Value::parse(v).to_source(), v);
        }
    }

    #[test]
    fn exports_and_errors() {
        let d = parse(
            "component A { }\nexport { A }\nexport { B as C } from \"b.slint\";\nexport * from \"c.slint\";\n",
        );
        assert!(d.components[0].exported);
        assert_eq!(d.re_exports[0].names, [("B".to_owned(), "C".to_owned())]);
        assert!(d.re_exports[1].all);
        let bad = parse("export component A inherits Window { Button { text: ; } }");
        assert!(!bad.errors.is_empty());
        assert_eq!(bad.components.len(), 1, "keeps what it could read");
    }
}
