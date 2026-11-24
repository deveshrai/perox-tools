//! Designer operations as minimal text edits on a `.slint` document. Setting a property
//! replaces only its value; adding an element inserts lines at the right indentation;
//! nothing else (comments, blank lines, hand formatting) changes.
//!
//! Elements are addressed by component name and path (see [`crate::syntax`]).

use std::ops::Range;

use crate::syntax::{Component, Element, parse};

/// Replace `range` (bytes) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    NoSuchComponent(String),
    NoSuchElement,
    /// Moving an element into itself or one of its children.
    IntoItself,
    InvalidName(String),
    /// The component's root can't be removed, moved or named.
    Root,
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::NoSuchComponent(c) => write!(f, "no component `{c}` in this file"),
            EditError::NoSuchElement => write!(f, "no such element"),
            EditError::IntoItself => write!(f, "an element can't be moved into itself"),
            EditError::InvalidName(n) => write!(
                f,
                "`{n}` isn't a valid name (letters, digits, - and _; not starting with a digit)"
            ),
            EditError::Root => write!(f, "that isn't possible on the component's root"),
        }
    }
}

impl std::error::Error for EditError {}

/// Applies edits (non-overlapping, any order) to `src`.
pub fn apply(src: &str, edits: &[TextEdit]) -> String {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    let mut out = src.to_owned();
    for e in sorted {
        out.replace_range(e.range.clone(), &e.text);
    }
    out
}

/// A Slint identifier: letters, digits, `_`, `-`; not starting with a digit or `-`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !matches!(name, "root" | "parent" | "self" | "true" | "false")
}

fn check(names: &[&str]) -> Result<(), EditError> {
    match names.iter().find(|n| !is_valid_name(n)) {
        Some(n) => Err(EditError::InvalidName((*n).into())),
        None => Ok(()),
    }
}

/// `kind` may be dotted (`Lib.Widget`).
fn check_kind(kind: &str) -> Result<(), EditError> {
    if kind.split('.').all(is_valid_name) {
        Ok(())
    } else {
        Err(EditError::InvalidName(kind.into()))
    }
}

fn line_start(src: &str, offset: usize) -> usize {
    src[..offset].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(src: &str, offset: usize) -> usize {
    src[offset..].find('\n').map_or(src.len(), |i| offset + i)
}

fn indent_at(src: &str, offset: usize) -> String {
    let start = line_start(src, offset);
    src[start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// True if only whitespace precedes `offset` on its line.
fn starts_line(src: &str, offset: usize) -> bool {
    src[line_start(src, offset)..offset].trim().is_empty()
}

/// The indentation unit used in this file (first indented line), default 4 spaces.
fn indent_unit(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        })
        .find(|i| !i.is_empty())
        .unwrap_or_else(|| "    ".into())
}

fn single_line(src: &str, el: &Element) -> bool {
    !src[el.open..el.close].contains('\n')
}

/// Indentation for members of `el`.
fn member_indent(src: &str, el: &Element) -> String {
    match el
        .member_spans()
        .into_iter()
        .map(|r| r.start)
        .find(|o| starts_line(src, *o))
    {
        Some(o) => indent_at(src, o),
        None => indent_at(src, el.span.start) + &indent_unit(src),
    }
}

/// Inserts a member line (`name: value;`, a handler) into `el`: after its last property,
/// handler or declaration that comes before its child elements.
fn insert_member(src: &str, el: &Element, line: &str) -> TextEdit {
    // Properties read best before the element's handlers.
    let first_child = el
        .children
        .iter()
        .map(|c| c.span.start)
        .min()
        .unwrap_or(usize::MAX);
    let first_handler = (!line.contains("=>"))
        .then(|| el.handlers.iter().map(|h| h.span.start).min())
        .flatten()
        .filter(|h| *h < first_child);
    if let Some(h) = first_handler {
        if single_line(src, el) {
            return TextEdit {
                range: h..h,
                text: format!("{line} "),
            };
        }
        if starts_line(src, h) {
            let at = line_start(src, h);
            return TextEdit {
                range: at..at,
                text: format!("{}{line}\n", indent_at(src, h)),
            };
        }
    }
    if single_line(src, el) {
        // `Button { text: "OK"; }` → `Button { text: "OK"; enabled: false; }`
        let inner = src[el.open + 1..el.close].trim_end();
        let at = el.open + 1 + inner.len();
        return TextEdit {
            range: at..el.close,
            text: format!(" {line} "),
        };
    }
    let first_child = el
        .children
        .iter()
        .map(|c| c.span.start)
        .min()
        .unwrap_or(usize::MAX);
    let last = el
        .member_spans()
        .into_iter()
        .filter(|r| r.start < first_child && !el.children.iter().any(|c| c.span == *r))
        .map(|r| r.end)
        .max();
    let indent = member_indent(src, el);
    let at = match last {
        Some(end) => line_end(src, end),
        None => el.open + 1,
    };
    TextEdit {
        range: at..at,
        text: format!("\n{indent}{line}"),
    }
}

/// Deletes a member's text: its whole line if it's alone there, else just itself and the
/// space after it.
fn delete_member(src: &str, span: Range<usize>) -> TextEdit {
    let ls = line_start(src, span.start);
    let le = line_end(src, span.end);
    let after = src[span.end..le].trim();
    if src[ls..span.start].trim().is_empty() && (after.is_empty() || after.starts_with("//")) {
        let end = if le < src.len() { le + 1 } else { le };
        return TextEdit {
            range: ls..end,
            text: String::new(),
        };
    }
    let rest = &src[span.end..];
    let end = span.end + (rest.len() - rest.trim_start_matches(' ').len());
    TextEdit {
        range: span.start..end,
        text: String::new(),
    }
}

fn component<'a>(doc: &'a crate::Document, name: &str) -> Result<&'a Component, EditError> {
    doc.component(name)
        .ok_or_else(|| EditError::NoSuchComponent(name.into()))
}

fn element<'a>(c: &'a Component, path: &[usize]) -> Result<&'a Element, EditError> {
    c.at(path).ok_or(EditError::NoSuchElement)
}

/// Sets (or adds) a property to an expression (`"OK"`, `12px`, `root.width / 2`).
pub fn set_property(
    src: &str,
    comp: &str,
    path: &[usize],
    name: &str,
    value: &str,
) -> Result<Vec<TextEdit>, EditError> {
    check(&[name])?;
    let doc = parse(src);
    let c = component(&doc, comp)?;
    let el = element(c, path)?;
    // A property the component declares on itself: its declaration holds the value
    // (`in property <string> title: "Card";`); a second binding wouldn't compile.
    if path.is_empty()
        && let Some(decl) = c.property(name).filter(|d| d.alias.is_none())
        && !el.props.iter().any(|p| p.name == name)
    {
        return Ok(vec![declaration_value(src, &decl.span, Some(value))]);
    }
    Ok(vec![match el.props.iter().find(|p| p.name == name) {
        Some(p) => TextEdit {
            range: p.value_span.clone(),
            text: value.into(),
        },
        None => insert_member(src, el, &format!("{name}: {value};")),
    }])
}

/// The edit that sets (or, with `None`, removes) the value of a property declaration:
/// `in property <string> title: "Card";` ↔ `in property <string> title;`.
fn declaration_value(src: &str, span: &Range<usize>, value: Option<&str>) -> TextEdit {
    let text = &src[span.clone()];
    let body = text.trim_end().trim_end_matches(';');
    let after_type = body.rfind('>').map_or(0, |i| i + 1);
    let end = span.start + body.len();
    match body[after_type..].find(':') {
        Some(colon) => {
            let start = span.start + after_type + colon;
            TextEdit {
                range: start..end,
                text: value.map(|v| format!(": {v}")).unwrap_or_default(),
            }
        }
        None => TextEdit {
            range: end..end,
            text: value.map(|v| format!(": {v}")).unwrap_or_default(),
        },
    }
}

pub fn remove_property(
    src: &str,
    comp: &str,
    path: &[usize],
    name: &str,
) -> Result<Vec<TextEdit>, EditError> {
    let doc = parse(src);
    let c = component(&doc, comp)?;
    let el = element(c, path)?;
    if path.is_empty()
        && !el.props.iter().any(|p| p.name == name)
        && let Some(decl) = c.property(name).filter(|d| d.alias.is_none())
    {
        return Ok(vec![declaration_value(src, &decl.span, None)]);
    }
    Ok(el
        .props
        .iter()
        .find(|p| p.name == name)
        .map(|p| delete_member(src, p.span.clone()))
        .into_iter()
        .collect())
}

/// Sets what a callback does: `event(params) => { body }`.
pub fn set_handler(
    src: &str,
    comp: &str,
    path: &[usize],
    event: &str,
    params: &[&str],
    body: &str,
) -> Result<Vec<TextEdit>, EditError> {
    // `changed <property>`: runs when the property changes.
    check(&[event.strip_prefix("changed ").unwrap_or(event)])?;
    check(params)?;
    let doc = parse(src);
    let el = element(component(&doc, comp)?, path)?;
    let args = if params.is_empty() {
        String::new()
    } else {
        format!("({})", params.join(", "))
    };
    let text = format!("{event}{args} => {{ {} }}", body.trim());
    Ok(vec![match el.handlers.iter().find(|h| h.event == event) {
        Some(h) => TextEdit {
            range: h.span.clone(),
            text,
        },
        None => insert_member(src, el, &text),
    }])
}

pub fn remove_handler(
    src: &str,
    comp: &str,
    path: &[usize],
    event: &str,
) -> Result<Vec<TextEdit>, EditError> {
    let doc = parse(src);
    let el = element(component(&doc, comp)?, path)?;
    Ok(el
        .handlers
        .iter()
        .find(|h| h.event == event)
        .map(|h| delete_member(src, h.span.clone()))
        .into_iter()
        .collect())
}

/// Changes an element's type (`VerticalBox` → `Rectangle`), keeping its id, members and
/// formatting.
pub fn set_kind(
    src: &str,
    comp: &str,
    path: &[usize],
    kind: &str,
) -> Result<Vec<TextEdit>, EditError> {
    check_kind(kind)?;
    let doc = parse(src);
    let el = element(component(&doc, comp)?, path)?;
    Ok(vec![TextEdit {
        range: el.kind_span.clone(),
        text: kind.into(),
    }])
}

/// Names (or renames) an element; `None` removes the name.
pub fn set_id(
    src: &str,
    comp: &str,
    path: &[usize],
    id: Option<&str>,
) -> Result<Vec<TextEdit>, EditError> {
    if let Some(n) = id {
        check(&[n])?;
    }
    if path.is_empty() {
        return Err(EditError::Root);
    }
    let doc = parse(src);
    let el = element(component(&doc, comp)?, path)?;
    Ok(match (&el.id_span, id) {
        (Some(s), Some(n)) => vec![TextEdit {
            range: s.clone(),
            text: n.into(),
        }],
        (Some(s), None) => vec![TextEdit {
            range: s.start..el.kind_span.start,
            text: String::new(),
        }],
        (None, Some(n)) => vec![TextEdit {
            range: el.kind_span.start..el.kind_span.start,
            text: format!("{n} := "),
        }],
        (None, None) => Vec::new(),
    })
}

/// A new element's source at `indent`: one line if it has at most one property.
pub fn element_source(
    kind: &str,
    id: Option<&str>,
    props: &[(String, String)],
    indent: &str,
    unit: &str,
) -> String {
    let head = match id {
        Some(i) => format!("{i} := {kind}"),
        None => kind.to_owned(),
    };
    match props {
        [] => format!("{indent}{head} {{ }}"),
        [(n, v)] => format!("{indent}{head} {{ {n}: {v}; }}"),
        many => {
            let mut s = format!("{indent}{head} {{\n");
            for (n, v) in many {
                s.push_str(&format!("{indent}{unit}{n}: {v};\n"));
            }
            s.push_str(&format!("{indent}}}"));
            s
        }
    }
}

/// Re-indents a block so its first line starts at `to`, and later lines lose `from`.
fn reindent(block: &str, from: &str, to: &str) -> String {
    block
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                format!("{to}{}", l.trim_start())
            } else if let Some(rest) = l.strip_prefix(from) {
                format!("{to}{rest}")
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where a new child goes in `parent` at `index`, and the text to put there.
fn insert_child_edit(src: &str, parent: &Element, index: usize, body: &str) -> TextEdit {
    let indent = member_indent(src, parent);
    let text = reindent(body, "", &indent);
    if single_line(src, parent) {
        // Open the one-line parent up: what it had goes on its own line, then the child,
        // then `}`.
        let parent_indent = indent_at(src, parent.span.start);
        let inner = src[parent.open + 1..parent.close].trim();
        let had = if inner.is_empty() {
            String::new()
        } else {
            format!("\n{indent}{inner}")
        };
        return TextEdit {
            range: parent.open + 1..parent.close,
            text: format!("{had}\n{text}\n{parent_indent}"),
        };
    }
    match parent.children.get(index) {
        // Before an existing child: on the line above it and its comments.
        Some(next) if starts_line(src, next.span.start) => {
            let mut at = line_start(src, next.span.start);
            while at > 0 {
                let prev = line_start(src, at - 1);
                let line = src[prev..at].trim();
                if line.starts_with("//") || (line.starts_with("/*") && line.ends_with("*/")) {
                    at = prev;
                } else {
                    break;
                }
            }
            TextEdit {
                range: at..at,
                text: format!("{text}\n"),
            }
        }
        Some(next) => TextEdit {
            range: next.span.start..next.span.start,
            text: format!("{}\n{indent}", text.trim_start()),
        },
        // At the end: before the closing brace's line.
        None => {
            if starts_line(src, parent.close) {
                let close_line = line_start(src, parent.close);
                TextEdit {
                    range: close_line..close_line,
                    text: format!("{text}\n"),
                }
            } else {
                TextEdit {
                    range: parent.close..parent.close,
                    text: format!("\n{text}\n{}", indent_at(src, parent.span.start)),
                }
            }
        }
    }
}

/// Adds an element as child `index` of `parent` (`usize::MAX`: at the end). Property
/// values are expressions.
pub fn add_element(
    src: &str,
    comp: &str,
    parent: &[usize],
    index: usize,
    kind: &str,
    id: Option<&str>,
    props: &[(String, String)],
) -> Result<Vec<TextEdit>, EditError> {
    check_kind(kind)?;
    if let Some(i) = id {
        check(&[i])?;
    }
    let doc = parse(src);
    let p = element(component(&doc, comp)?, parent)?;
    let body = element_source(kind, id, props, "", &indent_unit(src));
    Ok(vec![insert_child_edit(
        src,
        p,
        index.min(p.children.len()),
        &body,
    )])
}

/// Makes the element at `path` repeated or conditional: `header` is `for item in model`
/// or `if condition` (written before it with ` : `), or `None` for a plain element again.
pub fn set_header(
    src: &str,
    comp: &str,
    path: &[usize],
    header: Option<&str>,
) -> Result<Vec<TextEdit>, EditError> {
    if path.is_empty() {
        return Err(EditError::NoSuchElement);
    }
    let doc = parse(src);
    let el = element(component(&doc, comp)?, path)?;
    let start = el.id_span.as_ref().map_or(el.kind_span.start, |r| r.start);
    let text = match header.map(str::trim).filter(|h| !h.is_empty()) {
        Some(h) => {
            if !(h.starts_with("for ") || h.starts_with("if ")) {
                return Err(EditError::NoSuchElement);
            }
            format!("{h} : ")
        }
        None => String::new(),
    };
    Ok(vec![TextEdit {
        range: el.span.start..start,
        text,
    }])
}

/// Puts `@children` into `parent` at child position `index`: where a component shows the
/// elements a form places inside it.
pub fn add_children_slot(
    src: &str,
    comp: &str,
    parent: &[usize],
    index: usize,
) -> Result<Vec<TextEdit>, EditError> {
    let doc = parse(src);
    let p = element(component(&doc, comp)?, parent)?;
    Ok(vec![insert_child_edit(
        src,
        p,
        index.min(p.children.len()),
        "@children",
    )])
}

/// An element's source with its indentation removed (the first line starts at column 0,
/// later lines relative to it): what copy puts on the clipboard.
pub fn element_block(src: &str, el: &Element) -> String {
    reindent(&src[el.span.clone()], &indent_at(src, el.span.start), "")
}

/// Inserts `block` (one or more elements' source, as [`element_block`] gives it) as
/// child `index` of `parent` (`usize::MAX`: at the end).
pub fn insert_source(
    src: &str,
    comp: &str,
    parent: &[usize],
    index: usize,
    block: &str,
) -> Result<Vec<TextEdit>, EditError> {
    let doc = parse(src);
    let p = element(component(&doc, comp)?, parent)?;
    let unit = indent_unit(src);
    // Re-indent with this file's unit if the block used another.
    let block = block.trim_end();
    Ok(vec![insert_child_edit(
        src,
        p,
        index.min(p.children.len()),
        &block.replace('\t', &unit),
    )])
}

pub fn remove_element(src: &str, comp: &str, path: &[usize]) -> Result<Vec<TextEdit>, EditError> {
    if path.is_empty() {
        return Err(EditError::Root);
    }
    let doc = parse(src);
    let c = component(&doc, comp)?;
    let el = element(c, path)?;
    let mut edits = vec![delete_member(src, el.span.clone())];
    // Root properties that alias something inside (`<=> name_edit.text`) would dangle.
    let mut ids = Vec::new();
    collect_ids(el, &mut ids);
    for p in &c.properties {
        let target = p.alias.as_deref().and_then(|a| a.split('.').next());
        if target.is_some_and(|t| ids.contains(&crate::syntax::normalize(t))) {
            edits.push(delete_member(src, p.span.clone()));
        }
    }
    Ok(edits)
}

fn collect_ids(el: &Element, out: &mut Vec<String>) {
    if let Some(id) = &el.id {
        out.push(crate::syntax::normalize(id));
    }
    for c in &el.children {
        collect_ids(c, out);
    }
}

/// Moves an element to child `index` of `to_parent` (indices as before the move).
pub fn move_element(
    src: &str,
    comp: &str,
    from: &[usize],
    to_parent: &[usize],
    index: usize,
) -> Result<Vec<TextEdit>, EditError> {
    if from.is_empty() {
        return Err(EditError::Root);
    }
    if to_parent.starts_with(from) {
        return Err(EditError::IntoItself);
    }
    let doc = parse(src);
    let c = component(&doc, comp)?;
    let el = element(c, from)?;
    let parent = element(c, to_parent)?;
    if from[..from.len() - 1] == *to_parent
        && (index == from[from.len() - 1] || index == from[from.len() - 1] + 1)
    {
        return Ok(Vec::new());
    }
    let block = &src[el.span.clone()];
    let body = reindent(block, &indent_at(src, el.span.start), "");
    let remove = delete_member(src, el.span.clone());
    let insert = insert_child_edit(src, parent, index.min(parent.children.len()), &body);
    if remove.range.start < insert.range.end && insert.range.start < remove.range.end {
        return Err(EditError::IntoItself);
    }
    Ok(vec![remove, insert])
}

/// Makes sure `name` is imported from `from` (`"std-widgets.slint"`).
pub fn ensure_import(src: &str, name: &str, from: &str) -> Vec<TextEdit> {
    let doc = parse(src);
    if doc.imports_name(name, from) {
        return Vec::new();
    }
    if let Some(close) = doc
        .imports
        .iter()
        .find(|i| i.from == from)
        .and_then(|i| i.list_close)
    {
        let inner_end = src[..close].trim_end().len();
        let sep = if src[..inner_end].ends_with('{') || src[..inner_end].ends_with(',') {
            " "
        } else {
            ", "
        };
        return vec![TextEdit {
            range: inner_end..inner_end,
            text: format!("{sep}{name}"),
        }];
    }
    let line = format!("import {{ {name} }} from \"{from}\";\n");
    let at = doc
        .imports
        .iter()
        .map(|i| i.span.end)
        .max()
        .map_or(0, |end| (line_end(src, end) + 1).min(src.len()));
    let text = if at == src.len() && !src.is_empty() && !src.ends_with('\n') {
        format!("\n{line}")
    } else {
        line
    };
    vec![TextEdit {
        range: at..at,
        text,
    }]
}

/// Imports a file for its side effect, `import "fonts/Inter.ttf";` (fonts), after the
/// other imports. Does nothing if it's imported already.
pub fn ensure_file_import(src: &str, file: &str) -> Vec<TextEdit> {
    let doc = parse(src);
    if doc.imports.iter().any(|i| i.from == file) {
        return Vec::new();
    }
    let line = format!("import {};\n", crate::syntax::quote(file));
    let at = doc
        .imports
        .iter()
        .map(|i| i.span.end)
        .max()
        .map_or(0, |end| (line_end(src, end) + 1).min(src.len()));
    let text = if at == src.len() && !src.is_empty() && !src.ends_with('\n') {
        format!("\n{line}")
    } else {
        line
    };
    vec![TextEdit {
        range: at..at,
        text,
    }]
}

/// Declares `callback name(params);` on a component, after its other callbacks.
/// Does nothing if it exists.
pub fn add_callback(
    src: &str,
    comp: &str,
    name: &str,
    params: &[(Option<&str>, &str)],
) -> Result<Vec<TextEdit>, EditError> {
    check(&[name])?;
    let doc = parse(src);
    let c = component(&doc, comp)?;
    if c.callback(name).is_some() {
        return Ok(Vec::new());
    }
    let args = params
        .iter()
        .map(|(n, t)| match n {
            Some(n) => format!("{n}: {t}"),
            None => (*t).to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let line = format!("callback {name}({args});");
    let after = c
        .callbacks
        .iter()
        .map(|cb| cb.span.end)
        .max()
        .or_else(|| c.properties.iter().map(|p| p.span.end).max());
    Ok(vec![declare(src, c, after, &line)])
}

/// Inserts a declaration line at the top of a component: after `after`, else right
/// after `{`.
fn declare(src: &str, c: &Component, after: Option<usize>, line: &str) -> TextEdit {
    let root = &c.root;
    if single_line(src, root) {
        return insert_member(src, root, line);
    }
    let indent = member_indent(src, root);
    let at = match after {
        Some(end) => line_end(src, end),
        None => root.open + 1,
    };
    TextEdit {
        range: at..at,
        text: format!("\n{indent}{line}"),
    }
}

/// Exposes `target` (`name_edit.text`) to code as `in-out property <ty> name <=> target;`,
/// after the component's other properties. Does nothing if `name` exists.
pub fn expose_property(
    src: &str,
    comp: &str,
    name: &str,
    ty: &str,
    target: &str,
) -> Result<Vec<TextEdit>, EditError> {
    check(&[name])?;
    let doc = parse(src);
    let c = component(&doc, comp)?;
    if c.property(name).is_some() {
        return Ok(Vec::new());
    }
    let line = format!("in-out property <{ty}> {name} <=> {target};");
    let after = c
        .properties
        .iter()
        .map(|p| p.span.end)
        .max()
        .or_else(|| c.callbacks.iter().map(|cb| cb.span.end).max());
    Ok(vec![declare(src, c, after, &line)])
}

/// Declares a root property with an optional initial value:
/// `in-out property <ty> name: init;`.
pub fn add_property(
    src: &str,
    comp: &str,
    name: &str,
    ty: &str,
    init: Option<&str>,
) -> Result<Vec<TextEdit>, EditError> {
    declare_property(src, comp, "in-out", name, ty, init)
}

/// Declares a root property: `<visibility> property <ty> name: init;` (`in` for what
/// users of a component set, `out` for what it reports, `in-out` for both, `private`).
pub fn declare_property(
    src: &str,
    comp: &str,
    visibility: &str,
    name: &str,
    ty: &str,
    init: Option<&str>,
) -> Result<Vec<TextEdit>, EditError> {
    check(&[name])?;
    if !matches!(visibility, "in" | "out" | "in-out" | "private") {
        return Err(EditError::NoSuchElement);
    }
    let doc = parse(src);
    let c = component(&doc, comp)?;
    if c.property(name).is_some() {
        return Ok(Vec::new());
    }
    let vis = if visibility == "private" {
        String::new()
    } else {
        format!("{visibility} ")
    };
    let line = match init {
        Some(v) => format!("{vis}property <{ty}> {name}: {v};"),
        None => format!("{vis}property <{ty}> {name};"),
    };
    let after = c
        .properties
        .iter()
        .map(|p| p.span.end)
        .max()
        .or_else(|| c.callbacks.iter().map(|cb| cb.span.end).max());
    Ok(vec![declare(src, c, after, &line)])
}

/// The initial value written in a root property's declaration (`… name: value;`).
pub fn property_initial_value(src: &str, comp: &str, name: &str) -> Option<String> {
    let doc = parse(src);
    let p = component(&doc, comp).ok()?.property(name)?.clone();
    let text = src.get(p.span)?;
    let after_type = text.rfind('>').map_or(0, |i| i + 1);
    let (_, value) = text[after_type..].split_once(':')?;
    let value = value.trim().trim_end_matches(';').trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Removes a root property declaration.
pub fn remove_property_decl(src: &str, comp: &str, name: &str) -> Result<Vec<TextEdit>, EditError> {
    let doc = parse(src);
    let c = component(&doc, comp)?;
    Ok(c.property(name)
        .map(|p| delete_member(src, p.span.clone()))
        .into_iter()
        .collect())
}

pub fn remove_callback(src: &str, comp: &str, name: &str) -> Result<Vec<TextEdit>, EditError> {
    let doc = parse(src);
    let c = component(&doc, comp)?;
    Ok(c.callback(name)
        .map(|cb| delete_member(src, cb.span.clone()))
        .into_iter()
        .collect())
}

/// One edit that turns `old` into `new`: the changed middle, between the common prefix
/// and suffix. For multi-step designer operations computed on a working copy.
pub fn diff(old: &str, new: &str) -> Vec<TextEdit> {
    if old == new {
        return Vec::new();
    }
    let mut start = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }
    let max_suffix = old.len().min(new.len()) - start;
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    vec![TextEdit {
        range: start..old.len() - suffix,
        text: new[start..new.len() - suffix].to_owned(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "import { Button } from \"std-widgets.slint\";\n// header\nexport component Main inherits Window {\n    title: \"Hi\";   // keep me\n    callback go();\n\n    VerticalLayout {\n        a := Text { text: \"A\"; }\n        /* between */\n        b := Text { text: \"B\"; }\n    }\n}\n";

    #[test]
    fn removing_an_element_drops_properties_that_alias_it() {
        let src = "export component W inherits Window {\n    in-out property <string> name <=> name-edit.text;\n    in-out property <string> keep: \"\";\n    VerticalLayout {\n        HorizontalLayout {\n            name_edit := TextInput { }\n        }\n    }\n}\n";
        let out = apply(src, &remove_element(src, "W", &[0, 0]).unwrap());
        assert!(!out.contains("name <=>"), "{out}");
        assert!(out.contains("property <string> keep"));
        assert!(!out.contains("TextInput"));
    }

    fn run(edits: Result<Vec<TextEdit>, EditError>) -> String {
        let out = apply(SRC, &edits.unwrap());
        let d = parse(&out);
        assert!(d.errors.is_empty(), "{out}\n{:?}", d.errors);
        out
    }

    #[test]
    fn a_child_opens_a_one_line_parent_onto_lines() {
        let src = "export component W inherits Window {\n    MenuBar {\n        Menu { title: \"File\"; }\n    }\n}\n";
        let out = apply(
            src,
            &add_element(
                src,
                "W",
                &[0, 0],
                usize::MAX,
                "MenuItem",
                Some("open"),
                &[("title".into(), "\"Open\"".into())],
            )
            .unwrap(),
        );
        assert_eq!(
            out,
            "export component W inherits Window {\n    MenuBar {\n        Menu {\n            title: \"File\";\n            open := MenuItem { title: \"Open\"; }\n        }\n    }\n}\n"
        );
        let out = apply(
            src,
            &add_element(src, "W", &[0], usize::MAX, "Menu", None, &[]).unwrap(),
        );
        assert!(
            out.contains("        Menu { title: \"File\"; }\n        Menu { }\n"),
            "{out}"
        );
    }

    #[test]
    fn font_files_are_imported_once() {
        let out = run(Ok(ensure_file_import(SRC, "fonts/Inter.ttf")));
        assert!(
            out.starts_with(
                "import { Button } from \"std-widgets.slint\";\nimport \"fonts/Inter.ttf\";\n"
            ),
            "{out}"
        );
        let d = parse(&out);
        assert!(
            d.imports
                .iter()
                .any(|i| i.from == "fonts/Inter.ttf" && i.names.is_empty()),
            "{:?}",
            d.imports
        );
        assert!(ensure_file_import(&out, "fonts/Inter.ttf").is_empty());
    }

    #[test]
    fn repeated_elements_edit_like_any_other() {
        let src = "export component W inherits Window {\n    VerticalLayout {\n        for name in [\"a\", \"b\"] : Text { text: name; }\n        Rectangle { }\n    }\n}\n";
        let out = apply(
            src,
            &set_property(src, "W", &[0, 0], "color", "#ff0000").unwrap(),
        );
        assert!(
            out.contains("for name in [\"a\", \"b\"] : Text { text: name; color: #ff0000; }"),
            "{out}"
        );
        let out = apply(src, &remove_element(src, "W", &[0, 0]).unwrap());
        assert!(!out.contains("for "), "{out}");
        let out = apply(src, &move_element(src, "W", &[0, 0], &[0], 2).unwrap());
        assert!(out.contains("Rectangle { }\n        for name in"), "{out}");
        assert!(crate::parse(&out).errors.is_empty(), "{out}");
    }

    #[test]
    fn a_components_own_property_is_set_in_its_declaration() {
        let src = "export component Card inherits Rectangle {\n    in property <string> title: \"Card\";\n    in property <length> gap;\n    width: 10px;\n}\n";
        let out = apply(
            src,
            &set_property(src, "Card", &[], "title", "\"\"").unwrap(),
        );
        assert!(out.contains("in property <string> title: \"\";"), "{out}");
        let out = apply(
            &out,
            &set_property(&out, "Card", &[], "gap", "8px").unwrap(),
        );
        assert!(out.contains("in property <length> gap: 8px;"), "{out}");
        let out = apply(&out, &remove_property(&out, "Card", &[], "gap").unwrap());
        assert!(out.contains("in property <length> gap;"), "{out}");
        // Ordinary bindings are untouched.
        let out = apply(
            &out,
            &set_property(&out, "Card", &[], "width", "20px").unwrap(),
        );
        assert!(out.contains("width: 20px;"), "{out}");
        assert!(parse(&out).errors.is_empty(), "{out}");
    }

    #[test]
    fn headers_make_elements_repeat_or_depend() {
        let src = "export component W inherits Window {\n    VerticalLayout {\n        row := Text { text: \"x\"; }\n    }\n}\n";
        let out = apply(
            src,
            &set_header(src, "W", &[0, 0], Some("for item in root.items")).unwrap(),
        );
        assert!(
            out.contains("for item in root.items : row := Text {"),
            "{out}"
        );
        let d = parse(&out);
        assert!(d.errors.is_empty(), "{out}");
        assert_eq!(
            d.components[0].at(&[0, 0]).unwrap().header.as_deref(),
            Some("for item in root.items")
        );
        let out2 = apply(
            &out,
            &set_header(&out, "W", &[0, 0], Some("if root.show")).unwrap(),
        );
        assert!(
            out2.contains("        if root.show : row := Text {"),
            "{out2}"
        );
        let back = apply(&out2, &set_header(&out2, "W", &[0, 0], None).unwrap());
        assert_eq!(back, src);
        assert!(set_header(src, "W", &[0, 0], Some("while x")).is_err());
    }

    #[test]
    fn a_children_slot_goes_where_asked() {
        let src = "export component Card inherits Rectangle {\n    VerticalLayout {\n        Text { text: \"Title\"; }\n    }\n}\n";
        let out = apply(
            src,
            &add_children_slot(src, "Card", &[0], usize::MAX).unwrap(),
        );
        assert!(
            out.contains("Text { text: \"Title\"; }\n        @children\n    }"),
            "{out}"
        );
        let d = parse(&out);
        assert!(d.errors.is_empty(), "{out}");
        let (path, el) = d.components[0].children_slot().unwrap();
        assert_eq!(path, vec![0]);
        assert_eq!(el.slot, Some(1));
    }

    #[test]
    fn changing_a_value_touches_only_the_value() {
        let out = run(set_property(SRC, "Main", &[], "title", "\"Hello\""));
        assert_eq!(out, SRC.replace("\"Hi\"", "\"Hello\""));
    }

    #[test]
    fn new_property_goes_after_the_last_member_before_children() {
        let out = run(set_property(SRC, "Main", &[], "width", "400px"));
        assert!(
            out.contains("    callback go();\n    width: 400px;\n\n    VerticalLayout"),
            "{out}"
        );
        let out = run(set_property(SRC, "Main", &[0], "spacing", "8px"));
        assert!(
            out.contains("    VerticalLayout {\n        spacing: 8px;\n        a := Text"),
            "{out}"
        );
    }

    #[test]
    fn one_line_elements_stay_on_one_line() {
        let out = run(set_property(SRC, "Main", &[0, 0], "color", "#ff0000"));
        assert!(
            out.contains("a := Text { text: \"A\"; color: #ff0000; }"),
            "{out}"
        );
        let back = apply(
            &out,
            &remove_property(&out, "Main", &[0, 0], "color").unwrap(),
        );
        assert_eq!(back, SRC);
    }

    #[test]
    fn removing_a_property_removes_its_line() {
        let out = run(remove_property(SRC, "Main", &[], "title"));
        assert!(!out.contains("title") && !out.contains("keep me"), "{out}");
        assert!(out.contains("{\n    callback go();"), "{out}");
    }

    #[test]
    fn adding_elements_at_any_position() {
        let props = vec![("text".to_owned(), "\"Go\"".to_owned())];
        let out = run(add_element(
            SRC,
            "Main",
            &[0],
            1,
            "Button",
            Some("go-btn"),
            &props,
        ));
        assert!(out.contains("        a := Text { text: \"A\"; }\n        go-btn := Button { text: \"Go\"; }\n        /* between */\n        b := Text"), "{out}");
        let out = run(add_element(
            SRC,
            "Main",
            &[0],
            usize::MAX,
            "Rectangle",
            None,
            &[],
        ));
        assert!(
            out.contains("        b := Text { text: \"B\"; }\n        Rectangle { }\n    }"),
            "{out}"
        );
        let two = vec![
            ("text".to_owned(), "\"x\"".to_owned()),
            ("enabled".to_owned(), "false".to_owned()),
        ];
        let out = run(add_element(
            SRC,
            "Main",
            &[],
            usize::MAX,
            "Button",
            Some("ok"),
            &two,
        ));
        assert!(
            out.contains(
                "    ok := Button {\n        text: \"x\";\n        enabled: false;\n    }\n}"
            ),
            "{out}"
        );
    }

    #[test]
    fn adding_into_an_empty_or_one_line_container() {
        let src = "export component W inherits Window {\n    HorizontalLayout { }\n}\n";
        let out = apply(
            src,
            &add_element(src, "W", &[0], 0, "Text", None, &[]).unwrap(),
        );
        assert_eq!(
            out,
            "export component W inherits Window {\n    HorizontalLayout {\n        Text { }\n    }\n}\n"
        );
    }

    #[test]
    fn removing_and_moving_elements_keeps_the_rest() {
        let out = run(remove_element(SRC, "Main", &[0, 0]));
        assert_eq!(out, SRC.replace("        a := Text { text: \"A\"; }\n", ""));
        let out = run(move_element(SRC, "Main", &[0, 1], &[], usize::MAX));
        assert!(
            out.contains("    }\n    b := Text { text: \"B\"; }\n}"),
            "{out}"
        );
        assert!(out.contains("/* between */"));
        assert_eq!(
            move_element(SRC, "Main", &[0], &[0, 0], 0),
            Err(EditError::IntoItself)
        );
        assert_eq!(remove_element(SRC, "Main", &[]), Err(EditError::Root));
    }

    #[test]
    fn handlers_ids_and_kinds() {
        let out = run(set_handler(
            SRC,
            "Main",
            &[0, 1],
            "pointer-event",
            &["e"],
            "root.go();",
        ));
        assert!(
            out.contains("b := Text { text: \"B\"; pointer-event(e) => { root.go(); } }"),
            "{out}"
        );
        let doc = parse(&out);
        let h = doc.components[0]
            .at(&[0, 1])
            .unwrap()
            .handler("pointer-event")
            .unwrap();
        assert_eq!(h.forwards_to(), Some("go"));
        let out2 = apply(
            &out,
            &remove_handler(&out, "Main", &[0, 1], "pointer-event").unwrap(),
        );
        assert_eq!(out2, SRC);
        // `changed <property>` handlers.
        let out = run(set_handler(
            SRC,
            "Main",
            &[0, 1],
            "changed width",
            &[],
            "root.go();",
        ));
        assert!(out.contains("changed width => { root.go(); }"), "{out}");
        let doc = parse(&out);
        let b = doc.components[0].at(&[0, 1]).unwrap();
        assert_eq!(b.handler("changed width").unwrap().body, "root.go();");
        let again = apply(
            &out,
            &set_handler(&out, "Main", &[0, 1], "changed width", &[], "root.stop();").unwrap(),
        );
        assert!(
            again.contains("changed width => { root.stop(); }"),
            "{again}"
        );
        assert_eq!(again.matches("changed width").count(), 1);
        let out = run(set_id(SRC, "Main", &[0], Some("column")));
        assert!(out.contains("    column := VerticalLayout {"));
        let out = run(set_id(SRC, "Main", &[0, 0], None));
        assert!(out.contains("        Text { text: \"A\"; }"));
        assert!(matches!(
            set_id(SRC, "Main", &[0], Some("9x")),
            Err(EditError::InvalidName(_))
        ));
        let out = run(set_kind(SRC, "Main", &[0], "HorizontalLayout"));
        assert!(out.contains("    HorizontalLayout {\n"));
    }

    #[test]
    fn imports_and_callbacks() {
        let out = apply(SRC, &ensure_import(SRC, "LineEdit", "std-widgets.slint"));
        assert!(
            out.starts_with("import { Button, LineEdit } from \"std-widgets.slint\";\n"),
            "{out}"
        );
        assert!(ensure_import(&out, "Button", "std-widgets.slint").is_empty());
        let out = apply(SRC, &ensure_import(SRC, "Card", "card.slint"));
        assert!(out.starts_with("import { Button } from \"std-widgets.slint\";\nimport { Card } from \"card.slint\";\n// header"), "{out}");
        let bare = "export component A inherits Window { }";
        let out = apply(bare, &ensure_import(bare, "Button", "std-widgets.slint"));
        assert_eq!(
            out,
            "import { Button } from \"std-widgets.slint\";\nexport component A inherits Window { }"
        );

        let out = run(add_callback(
            SRC,
            "Main",
            "save-clicked",
            &[(Some("name"), "string")],
        ));
        assert!(
            out.contains("    callback go();\n    callback save-clicked(name: string);\n"),
            "{out}"
        );
        assert!(add_callback(SRC, "Main", "go", &[]).unwrap().is_empty());
        let out = run(remove_callback(SRC, "Main", "go"));
        assert!(!out.contains("callback"), "{out}");
    }

    #[test]
    fn copy_and_paste_source() {
        let doc = parse(SRC);
        let b = doc.components[0].at(&[0, 1]).unwrap();
        let block = element_block(SRC, b);
        assert_eq!(block, "b := Text { text: \"B\"; }");
        let out = run(insert_source(
            SRC,
            "Main",
            &[],
            usize::MAX,
            "c := Rectangle {\n    Text { }\n}",
        ));
        assert!(
            out.contains("    }\n    c := Rectangle {\n        Text { }\n    }\n}"),
            "{out}"
        );
    }

    #[test]
    fn exposing_properties() {
        let out = run(expose_property(SRC, "Main", "a_text", "string", "a.text"));
        assert!(
            out.contains("    callback go();\n    in-out property <string> a_text <=> a.text;\n"),
            "{out}"
        );
        let c = parse(&out).components.remove(0);
        assert_eq!(c.exposing("a.text").unwrap().name, "a_text");
        assert!(
            expose_property(&out, "Main", "a_text", "string", "a.text")
                .unwrap()
                .is_empty()
        );
        let back = apply(&out, &remove_property_decl(&out, "Main", "a_text").unwrap());
        assert_eq!(back, SRC);
    }

    #[test]
    fn diff_is_minimal_and_char_safe() {
        let a = "héllo wörld";
        let b = "héllo wörlds";
        assert_eq!(apply(a, &diff(a, b)), b);
        assert_eq!(apply(b, &diff(b, a)), a);
        assert!(diff(a, a).is_empty());
    }
}
