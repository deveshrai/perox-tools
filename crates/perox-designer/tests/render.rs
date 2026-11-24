//! Rendering a form to pixels on its own (what `designer.screenshot` saves).

use std::path::Path;

use perox_designer::preview::render_component;

const SOURCE: &str = r#"
export component Swatch inherits Window {
    preferred-width: 120px;
    preferred-height: 80px;
    background: #ffffff;
    Rectangle { x: 10px; y: 10px; width: 40px; height: 30px; background: #ff0000; }
}
"#;

#[test]
fn renders_at_the_preferred_size_and_scale() {
    let path = Path::new("/tmp/swatch.slint");
    let (w, h, px) = render_component(SOURCE, path, "Swatch", "fluent", false, None, 2.0).unwrap();
    assert_eq!((w, h), (240, 160));
    assert_eq!(px.len(), w * h * 4);
    let at = |x: usize, y: usize| &px[(y * w + x) * 4..(y * w + x) * 4 + 4];
    assert_eq!(at(40, 40), [255, 0, 0, 255], "the red rectangle, scaled");
    assert_eq!(at(200, 120), [255, 255, 255, 255], "the white window");

    let (w, h, _) = render_component(
        SOURCE,
        path,
        "Swatch",
        "cupertino",
        true,
        Some(eframe::egui::vec2(50.0, 20.0)),
        1.0,
    )
    .unwrap();
    assert_eq!((w, h), (50, 20));
}

#[test]
fn compile_errors_are_reported() {
    let err = render_component(
        "export component X inherits Window { nope }",
        Path::new("/tmp/x.slint"),
        "X",
        "fluent",
        false,
        None,
        1.0,
    )
    .unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn the_preview_shows_a_translation() {
    let root = std::env::temp_dir().join(format!("perox-tr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let ui = root.join("ui");
    std::fs::create_dir_all(&ui).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"notes\"\n").unwrap();
    let src = "export component Main inherits Window {\n    out property <string> label: @tr(\"Save\");\n    Text { text: root.label; }\n}\n";
    let file = ui.join("main.slint");
    std::fs::write(&file, src).unwrap();
    perox_slint::i18n::sync(&ui, "notes", Some("de")).unwrap();
    let po_path = perox_slint::i18n::po_path(&ui, "de", "notes");
    let mut po = perox_slint::i18n::parse_po(&std::fs::read_to_string(&po_path).unwrap());
    po.set("Main", "Save", "Speichern");
    let msgs = perox_slint::i18n::extract(src);
    std::fs::write(&po_path, perox_slint::i18n::write_po("de", &msgs, &po)).unwrap();

    let mut p = perox_designer::preview::Preview::default();
    p.update(src, &file, "Main", "fluent", false);
    assert_eq!(p.string_property("label").as_deref(), Some("Save"));
    p.language = Some((ui.join("translations"), "notes".into(), "de".into()));
    p.update(src, &file, "Main", "fluent", false);
    assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    assert_eq!(p.string_property("label").as_deref(), Some("Speichern"));
    p.language = None;
    p.update(src, &file, "Main", "fluent", false);
    assert_eq!(p.string_property("label").as_deref(), Some("Save"));
}
