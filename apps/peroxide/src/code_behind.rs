//! Finding and adding event handlers in a form's code-behind: the
//! `impl MainWindowHandlers for App { … }` block.

use std::ops::Range;

/// The byte range inside the braces of `impl <trait_name> for …`.
pub fn find_impl(src: &str, trait_name: &str) -> Option<Range<usize>> {
    let needle = format!("{trait_name} for ");
    let mut from = 0;
    while let Some(i) = src[from..].find(&needle) {
        let at = from + i;
        from = at + needle.len();
        // Must be `impl … Trait for`, on the same item (no `;` or `}` in between).
        let line_start = src[..at].rfind(['\n', ';', '}']).map_or(0, |p| p + 1);
        if !src[line_start..at].trim_start().starts_with("impl") || in_comment_or_string(src, at) {
            continue;
        }
        let open = at + src[at..].find('{')?;
        let close = matching_brace(src, open)?;
        return Some(open + 1..close);
    }
    None
}

/// The offset of `fn <method>` inside `body`.
pub fn find_method(src: &str, body: Range<usize>, method: &str) -> Option<usize> {
    let text = &src[body.clone()];
    let needle = format!("fn {method}");
    let mut from = 0;
    while let Some(i) = text[from..].find(&needle) {
        let at = from + i;
        let after = text[at + needle.len()..].chars().next();
        let before = text[..at].chars().next_back();
        if !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
            && !before.is_some_and(|c| c.is_alphanumeric() || c == '_')
        {
            return Some(body.start + at);
        }
        from = at + needle.len();
    }
    None
}

/// Where to insert a new handler and its text: just before the impl's closing brace,
/// indented like the other methods. Returns (offset, text, offset of the body's blank
/// line within the text).
pub fn new_method(
    src: &str,
    body: Range<usize>,
    form: &str,
    method: &str,
) -> (usize, String, usize) {
    let text = &src[body.clone()];
    let indent = text
        .lines()
        .find(|l| l.trim_start().starts_with("fn ") || l.trim_start().starts_with("pub fn "))
        .map(|l| l[..l.len() - l.trim_start().len()].to_owned())
        .unwrap_or_else(|| "    ".to_owned());
    let unit = if indent.starts_with('\t') {
        "\t"
    } else {
        "    "
    };
    // Insert at the start of the closing brace's line (or right before the brace
    // when the impl is on one line).
    let close = body.end;
    let line_start = src[..close].rfind('\n').map_or(0, |p| p + 1);
    let only_space_before = src[line_start..close].trim().is_empty();
    let has_methods = !text.trim().is_empty();
    let sep = if has_methods { "\n" } else { "" };
    let head =
        format!("{sep}{indent}fn {method}(&mut self, ui: &mut {form}<'_>) {{\n{indent}{unit}");
    let cursor = head.len();
    let body_text = format!("{head}\n{indent}}}\n");
    if only_space_before {
        (line_start, body_text, cursor)
    } else {
        (close, format!("\n{body_text}"), cursor + 1)
    }
}

/// Is `at` inside a comment or string? (Scans from the start; good enough for
/// source files of normal size.)
fn in_comment_or_string(src: &str, at: usize) -> bool {
    let mut state = Scan::default();
    for (i, _) in src.char_indices() {
        if i >= at {
            break;
        }
        state.step(src, i);
    }
    state.mode != Mode::Code
}

fn matching_brace(src: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut scan = Scan::default();
    for (i, c) in src.char_indices() {
        let code = scan.mode == Mode::Code;
        scan.step(src, i);
        if i < open || !code || scan.mode != Mode::Code {
            continue;
        }
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Mode {
    #[default]
    Code,
    Line,
    Block,
    Str,
    Char,
}

#[derive(Debug, Default)]
struct Scan {
    mode: Mode,
    skip: usize,
}

impl Scan {
    /// Advance over the char at byte `i`; `mode` is the state after it.
    fn step(&mut self, src: &str, i: usize) {
        if self.skip > 0 {
            self.skip -= 1;
            return;
        }
        let rest = &src.as_bytes()[i..];
        let c = rest[0];
        match self.mode {
            Mode::Code => match c {
                b'/' if rest.get(1) == Some(&b'/') => self.mode = Mode::Line,
                b'/' if rest.get(1) == Some(&b'*') => {
                    self.mode = Mode::Block;
                    self.skip = 1;
                }
                b'"' => self.mode = Mode::Str,
                // A char literal (`'x'`, `'\n'`), not a lifetime (`'a`).
                b'\'' if rest.get(2) == Some(&b'\'') || rest.get(1) == Some(&b'\\') => {
                    self.mode = Mode::Char
                }
                _ => {}
            },
            Mode::Line => {
                if c == b'\n' {
                    self.mode = Mode::Code;
                }
            }
            Mode::Block => {
                if c == b'*' && rest.get(1) == Some(&b'/') {
                    self.mode = Mode::Code;
                    self.skip = 1;
                }
            }
            Mode::Str => match c {
                b'\\' => self.skip = 1,
                b'"' => self.mode = Mode::Code,
                _ => {}
            },
            Mode::Char => match c {
                b'\\' => self.skip = 1,
                b'\'' => self.mode = Mode::Code,
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "use x::MainWindowHandlers;\n\n// impl MainWindowHandlers for Nope {\nstruct App;\n\nimpl MainWindowHandlers for App {\n    fn count_clicked(&mut self, ui: &mut MainWindow<'_>) {\n        let s = \"}\";\n        let c = '}';\n    }\n}\n\nfn main() {}\n";

    #[test]
    fn finds_the_impl_past_comments_and_braces_in_strings() {
        let body = find_impl(SRC, "MainWindowHandlers").unwrap();
        assert!(SRC[body.clone()].contains("count_clicked"));
        assert!(SRC[body.end..].starts_with("}\n\nfn main"));
        assert!(find_method(SRC, body.clone(), "count_clicked").is_some());
        assert!(find_method(SRC, body, "count").is_none());
    }

    #[test]
    fn adds_a_method_before_the_closing_brace() {
        let body = find_impl(SRC, "MainWindowHandlers").unwrap();
        let (at, text, cursor) = new_method(SRC, body, "MainWindow", "reset_clicked");
        let mut out = SRC.to_owned();
        out.insert_str(at, &text);
        assert!(out.contains("    }\n\n    fn reset_clicked(&mut self, ui: &mut MainWindow<'_>) {\n        \n    }\n}\n"), "{out}");
        assert_eq!(&out[at + cursor - 8..at + cursor], "        ");
        let body = find_impl(&out, "MainWindowHandlers").unwrap();
        assert!(find_method(&out, body, "reset_clicked").is_some());
    }

    #[test]
    fn empty_one_line_impl() {
        let src = "impl FormHandlers for App {}\n";
        let body = find_impl(src, "FormHandlers").unwrap();
        let (at, text, _) = new_method(src, body, "Form", "go");
        let mut out = src.to_owned();
        out.insert_str(at, &text);
        assert_eq!(
            out,
            "impl FormHandlers for App {\n    fn go(&mut self, ui: &mut Form<'_>) {\n        \n    }\n}\n"
        );
    }
}
