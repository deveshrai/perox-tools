//! Designer operations as minimal text edits. Setting a property replaces only its
//! value; adding a widget inserts lines at the right indentation; nothing else in the
//! file (comments, blank lines, hand formatting) changes.

use std::ops::Range;

use crate::syntax::{Element, File, Value, parse};

/// Replace `range` (bytes) with `text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    NoSuchElement,
    /// The source has errors where the edit would go; fix them first.
    Unparsable,
    /// Moving a widget into itself or one of its children.
    IntoItself,
    InvalidName(String),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::NoSuchElement => write!(f, "no such widget"),
            EditError::Unparsable => write!(f, "the form has syntax errors; fix them first"),
            EditError::IntoItself => write!(f, "a widget can't be moved into itself"),
            EditError::InvalidName(n) => write!(
                f,
                "`{n}` isn't a valid name (letters, digits, _; not starting with a digit)"
            ),
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

pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !matches!(name, "on" | "true" | "false")
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
    let first_member = el
        .props
        .iter()
        .map(|p| p.span.start)
        .chain(el.handlers.iter().map(|h| h.span.start))
        .chain(el.children.iter().map(|c| c.span.start))
        .filter(|o| starts_line(src, *o))
        .min();
    match first_member {
        Some(o) => indent_at(src, o),
        None => indent_at(src, el.span.start) + &indent_unit(src),
    }
}

/// Inserts a member line (property, handler) into `el`, after its last property or handler.
fn insert_member(src: &str, el: &Element, line: &str) -> TextEdit {
    if single_line(src, el) {
        // `Label l { text: "" }` → `Label l { text: ""; color: #fff }`
        let inner = src[el.open + 1..el.close].trim_end();
        let at = el.open + 1 + inner.len();
        let sep = if inner.trim().is_empty() { " " } else { "; " };
        let trailing = if src[at..el.close].is_empty() {
            " "
        } else {
            ""
        };
        return TextEdit {
            range: at..at,
            text: format!("{sep}{line}{trailing}"),
        };
    }
    let indent = member_indent(src, el);
    let last = el
        .props
        .iter()
        .map(|p| p.span.end)
        .chain(el.handlers.iter().map(|h| h.span.end))
        .max();
    match last {
        // After the last property's line.
        Some(end) => {
            let at = line_end(src, end);
            TextEdit {
                range: at..at,
                text: format!("\n{indent}{line}"),
            }
        }
        // First member: right after `{`.
        None => {
            let at = el.open + 1;
            TextEdit {
                range: at..at,
                text: format!("\n{indent}{line}"),
            }
        }
    }
}

fn element<'a>(file: &'a File, path: &[usize]) -> Result<&'a Element, EditError> {
    file.at(path).ok_or(EditError::NoSuchElement)
}

/// Sets (or adds) a property.
pub fn set_property(
    src: &str,
    path: &[usize],
    name: &str,
    value: &Value,
) -> Result<Vec<TextEdit>, EditError> {
    if !is_valid_name(name) {
        return Err(EditError::InvalidName(name.into()));
    }
    let file = parse(src);
    let el = element(&file, path)?;
    let text = value.to_source();
    Ok(vec![match el.props.iter().find(|p| p.name == name) {
        Some(p) => TextEdit {
            range: p.value_span.clone(),
            text,
        },
        None => insert_member(src, el, &format!("{name}: {text}")),
    }])
}

/// Deletes a member's text: its whole line if it's alone there, else just itself.
fn delete_member(src: &str, span: Range<usize>) -> TextEdit {
    let ls = line_start(src, span.start);
    let le = line_end(src, span.end);
    let before = &src[ls..span.start];
    let after = &src[span.end..le];
    if before.trim().is_empty() && after.trim().trim_start_matches(';').trim().is_empty() {
        let end = if le < src.len() { le + 1 } else { le };
        TextEdit {
            range: ls..end,
            text: String::new(),
        }
    } else {
        // Also eat a `; ` separator next to it.
        let mut start = span.start;
        let mut end = span.end;
        if let Some(rest) = src[end..].strip_prefix(';') {
            end += 1 + (rest.len() - rest.trim_start_matches(' ').len());
        } else if src[..start].trim_end().ends_with(';') {
            start = src[..start].trim_end().len() - 1;
        }
        TextEdit {
            range: start..end,
            text: String::new(),
        }
    }
}

pub fn remove_property(src: &str, path: &[usize], name: &str) -> Result<Vec<TextEdit>, EditError> {
    let file = parse(src);
    let el = element(&file, path)?;
    Ok(el
        .props
        .iter()
        .find(|p| p.name == name)
        .map(|p| delete_member(src, p.span.clone()))
        .into_iter()
        .collect())
}

/// Wires an event to a method (`on clicked: save`), or rewires it.
pub fn set_handler(
    src: &str,
    path: &[usize],
    event: &str,
    method: &str,
) -> Result<Vec<TextEdit>, EditError> {
    for n in [event, method] {
        if !is_valid_name(n) {
            return Err(EditError::InvalidName(n.into()));
        }
    }
    let file = parse(src);
    let el = element(&file, path)?;
    Ok(vec![match el.handlers.iter().find(|h| h.event == event) {
        Some(h) => TextEdit {
            range: h.method_span.clone(),
            text: method.into(),
        },
        None => insert_member(src, el, &format!("on {event}: {method}")),
    }])
}

/// Names (or renames) a widget; `None` removes the name.
pub fn set_id(src: &str, path: &[usize], id: Option<&str>) -> Result<Vec<TextEdit>, EditError> {
    if let Some(n) = id
        && !is_valid_name(n)
    {
        return Err(EditError::InvalidName(n.into()));
    }
    let file = parse(src);
    let el = element(&file, path)?;
    Ok(vec![match (&el.id_span, id) {
        (Some(s), Some(n)) => TextEdit {
            range: s.clone(),
            text: n.into(),
        },
        (Some(s), None) => TextEdit {
            range: el.kind_span.end..s.end,
            text: String::new(),
        },
        (None, Some(n)) => TextEdit {
            range: el.kind_span.end..el.kind_span.end,
            text: format!(" {n}"),
        },
        (None, None) => return Ok(Vec::new()),
    }])
}

/// A new widget's source, at `indent`: one line if it has at most one property.
pub fn widget_source(
    kind: &str,
    id: Option<&str>,
    props: &[(String, Value)],
    indent: &str,
    unit: &str,
) -> String {
    let head = match id {
        Some(i) => format!("{kind} {i}"),
        None => kind.to_owned(),
    };
    match props {
        [] => format!("{indent}{head} {{}}"),
        [(n, v)] => format!("{indent}{head} {{ {n}: {} }}", v.to_source()),
        many => {
            let mut s = format!("{indent}{head} {{\n");
            for (n, v) in many {
                s.push_str(&format!("{indent}{unit}{n}: {}\n", v.to_source()));
            }
            s.push_str(&format!("{indent}}}"));
            s
        }
    }
}

/// Re-indents a block of source so its first line starts at `indent`.
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
        // Open the one-line parent up: the child goes on its own line before `}`.
        let parent_indent = indent_at(src, parent.span.start);
        let inner_end = parent.open + 1 + src[parent.open + 1..parent.close].trim_end().len();
        return TextEdit {
            range: inner_end..parent.close,
            text: format!("\n{text}\n{parent_indent}"),
        };
    }
    match parent.children.get(index) {
        // Before an existing child: on the line above it.
        Some(next) if starts_line(src, next.span.start) => {
            // Above the next widget and any comment lines that belong to it.
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
            let close_line = line_start(src, parent.close);
            if starts_line(src, parent.close) {
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

/// Adds a widget as child `index` of the element at `parent` (`usize::MAX`: at the end).
pub fn add_widget(
    src: &str,
    parent: &[usize],
    index: usize,
    kind: &str,
    id: Option<&str>,
    props: &[(String, Value)],
) -> Result<Vec<TextEdit>, EditError> {
    for n in std::iter::once(kind).chain(id) {
        if !is_valid_name(n) {
            return Err(EditError::InvalidName(n.into()));
        }
    }
    let file = parse(src);
    let p = element(&file, parent)?;
    let body = widget_source(kind, id, props, "", &indent_unit(src));
    Ok(vec![insert_child_edit(
        src,
        p,
        index.min(p.children.len()),
        &body,
    )])
}

pub fn remove_widget(src: &str, path: &[usize]) -> Result<Vec<TextEdit>, EditError> {
    if path.is_empty() {
        return Err(EditError::NoSuchElement); // the root stays
    }
    let file = parse(src);
    let el = element(&file, path)?;
    Ok(vec![delete_member(src, el.span.clone())])
}

/// Moves a widget to child `index` of `to_parent` (indices as before the move).
pub fn move_widget(
    src: &str,
    from: &[usize],
    to_parent: &[usize],
    index: usize,
) -> Result<Vec<TextEdit>, EditError> {
    if from.is_empty() {
        return Err(EditError::NoSuchElement);
    }
    if to_parent.starts_with(from) {
        return Err(EditError::IntoItself);
    }
    let file = parse(src);
    let el = element(&file, from)?;
    let parent = element(&file, to_parent)?;
    // Same place: nothing to do.
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

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "// header comment\nWindow main {\n    title: \"Hi\"   // keep me\n\n    VBox {\n        Label a { text: \"A\" }\n        /* between */\n        Label b { text: \"B\" }\n    }\n}\n";

    fn run(edits: Result<Vec<TextEdit>, EditError>) -> String {
        let out = apply(SRC, &edits.unwrap());
        let f = parse(&out);
        assert!(f.errors.is_empty(), "{out}\n{:?}", f.errors);
        out
    }

    #[test]
    fn changing_a_value_touches_only_the_value() {
        let out = run(set_property(SRC, &[], "title", &Value::Str("Hello".into())));
        assert_eq!(out, SRC.replace("\"Hi\"", "\"Hello\""));
    }

    #[test]
    fn new_property_goes_after_the_last_one_with_the_same_indent() {
        let out = run(set_property(SRC, &[], "size", &Value::Size(800.0, 600.0)));
        assert!(
            out.contains("    title: \"Hi\"   // keep me\n    size: 800 x 600\n"),
            "{out}"
        );
        let out = run(set_property(SRC, &[0], "spacing", &Value::Num(8.0)));
        assert!(
            out.contains("    VBox {\n        spacing: 8\n        Label a"),
            "{out}"
        );
    }

    #[test]
    fn one_line_widgets_stay_on_one_line() {
        let out = run(set_property(
            SRC,
            &[0, 0],
            "color",
            &Value::Color([255, 0, 0, 255]),
        ));
        assert!(
            out.contains("Label a { text: \"A\"; color: #ff0000 }"),
            "{out}"
        );
        let back = apply(&out, &remove_property(&out, &[0, 0], "color").unwrap());
        assert_eq!(back, SRC);
    }

    #[test]
    fn removing_a_property_removes_its_line_or_its_separator() {
        let src = "W {\n    a: 1\n    b: 2\n}\n";
        assert_eq!(
            apply(src, &remove_property(src, &[], "a").unwrap()),
            "W {\n    b: 2\n}\n"
        );
        let src = "L { a: 1; b: 2 }";
        assert_eq!(
            apply(src, &remove_property(src, &[], "b").unwrap()),
            "L { a: 1 }"
        );
        assert_eq!(
            apply(src, &remove_property(src, &[], "a").unwrap()),
            "L { b: 2 }"
        );
    }

    #[test]
    fn adding_widgets_at_any_position() {
        let props = vec![("text".to_owned(), Value::Str("Go".into()))];
        let out = run(add_widget(SRC, &[0], 1, "Button", Some("go"), &props));
        assert!(out.contains("        Label a { text: \"A\" }\n        Button go { text: \"Go\" }\n        /* between */\n        Label b"), "{out}");
        let out = run(add_widget(SRC, &[0], usize::MAX, "Spacer", None, &[]));
        assert!(
            out.contains("        Label b { text: \"B\" }\n        Spacer {}\n    }"),
            "{out}"
        );
        let two = vec![
            ("text".to_owned(), Value::Str("x".into())),
            ("enabled".to_owned(), Value::Bool(false)),
        ];
        let out = run(add_widget(SRC, &[], usize::MAX, "Button", Some("ok"), &two));
        assert!(
            out.contains("    Button ok {\n        text: \"x\"\n        enabled: false\n    }\n}"),
            "{out}"
        );
    }

    #[test]
    fn adding_into_an_empty_or_one_line_container() {
        let src = "Window w {\n    HBox {}\n}\n";
        let out = apply(src, &add_widget(src, &[0], 0, "Label", None, &[]).unwrap());
        assert_eq!(out, "Window w {\n    HBox {\n        Label {}\n    }\n}\n");
    }

    #[test]
    fn removing_and_moving_widgets_keeps_the_rest() {
        let out = run(remove_widget(SRC, &[0, 0]));
        assert_eq!(out, SRC.replace("        Label a { text: \"A\" }\n", ""));
        // Move `b` to the window, after the VBox.
        let out = run(move_widget(SRC, &[0, 1], &[], usize::MAX));
        assert!(
            out.contains("    }\n    Label b { text: \"B\" }\n}"),
            "{out}"
        );
        assert!(out.contains("/* between */"), "comments survive");
        assert_eq!(
            move_widget(SRC, &[0], &[0, 0], 0),
            Err(EditError::IntoItself)
        );
    }

    #[test]
    fn handlers_and_names() {
        let out = run(set_handler(SRC, &[0, 1], "clicked", "on_b"));
        assert!(
            out.contains("Label b { text: \"B\"; on clicked: on_b }"),
            "{out}"
        );
        let out2 = apply(
            &out,
            &set_handler(&out, &[0, 1], "clicked", "other").unwrap(),
        );
        assert!(out2.contains("on clicked: other"));
        let out = run(set_id(SRC, &[0], Some("column")));
        assert!(out.contains("    VBox column {"));
        let out = run(set_id(SRC, &[0, 0], None));
        assert!(out.contains("        Label { text: \"A\" }"));
        assert!(matches!(
            set_id(SRC, &[0], Some("9x")),
            Err(EditError::InvalidName(_))
        ));
    }
}
