//! "Use from code": aliasing a property, and binding one Slint won't let you alias.

use perox_designer::{expose_property, unexpose_property};

const SRC: &str = "export component W inherits Window {\n    VerticalLayout {\n        label := Text { text: \"Hi\"; }\n        line := Path { commands: \"M 0 0 L 10 10\"; }\n    }\n}\n";

#[test]
fn aliases_ordinary_properties() {
    let out = expose_property(SRC, "W", &[0, 0], "text").unwrap();
    assert!(
        out.contains("in-out property <string> label_text <=> label.text;"),
        "{out}"
    );
    let back = unexpose_property(&out, "W", &[0, 0], "text").unwrap();
    assert_eq!(back, SRC);
}

#[test]
fn binds_path_commands_and_gives_the_value_back() {
    let out = expose_property(SRC, "W", &[0, 1], "commands").unwrap();
    assert!(
        out.contains("in-out property <string> line_commands: \"M 0 0 L 10 10\";"),
        "{out}"
    );
    assert!(out.contains("commands: root.line_commands;"), "{out}");
    let doc = perox_slint::parse(&out);
    let c = doc.component("W").unwrap();
    assert_eq!(
        c.exposing("line.commands").map(|p| p.name.as_str()),
        Some("line_commands")
    );
    // Exposing again changes nothing; un-exposing restores the literal.
    assert_eq!(
        expose_property(&out, "W", &[0, 1], "commands").unwrap(),
        out
    );
    let back = unexpose_property(&out, "W", &[0, 1], "commands").unwrap();
    assert!(back.contains("commands: \"M 0 0 L 10 10\";"), "{back}");
    assert!(!back.contains("line_commands"), "{back}");
}
