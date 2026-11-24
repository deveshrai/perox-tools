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
