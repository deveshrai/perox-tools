//! The designer driven headlessly with synthetic mouse and keyboard input. Each
//! gesture must turn into a minimal edit of the `.slint` text.

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use perox_designer::{Action, Designer};

/// `(component, method, params)` of each "open handler" request.
type Opened = Vec<(String, String, Vec<(String, String)>)>;

struct Harness {
    ctx: egui::Context,
    designer: Designer,
    src: String,
    opened: Opened,
    path: std::path::PathBuf,
}

impl Harness {
    fn new(src: &str) -> Self {
        Self::at(src, std::path::Path::new("main.slint"))
    }

    fn at(src: &str, path: &std::path::Path) -> Self {
        let mut h = Self {
            ctx: egui::Context::default(),
            designer: Designer::new(),
            src: src.to_owned(),
            opened: Vec::new(),
            path: path.to_owned(),
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 1200.0))),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let (designer, src, path) = (&mut self.designer, &self.src, &self.path);
        let mut out = self.ctx.run_ui(input, |ui| {
            actions = designer.show(ui, src, path);
        });
        out.textures_delta.clear();
        for a in actions {
            match a {
                Action::Edit(edits) => self.src = perox_slint::edit::apply(&self.src, &edits),
                Action::OpenHandler {
                    form,
                    method,
                    params,
                } => self.opened.push((form, method, params)),
                Action::Undo | Action::Redo => {}
            }
        }
    }

    fn rect(&self, path: &[usize]) -> Rect {
        self.designer
            .layout
            .elements
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("{path:?} not drawn"))
    }

    fn palette(&self, kind: &str) -> Rect {
        self.designer
            .layout
            .palette
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, r)| *r)
            .unwrap()
    }

    fn button(at: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    fn click(&mut self, at: Pos2) {
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![Self::button(at, true)]);
        self.frame(vec![Self::button(at, false)]);
        self.frame(vec![]);
    }

    fn drag(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![Self::button(from, true)]);
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            self.frame(vec![Event::PointerMoved(from + (to - from) * t)]);
        }
        self.frame(vec![Self::button(to, false)]);
        self.frame(vec![]);
        self.frame(vec![]);
    }

    fn key(&mut self, key: egui::Key) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
    }
}

const FORM: &str = "// keep this comment\nimport { Button, VerticalBox } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    title: \"T\";\n    width: 400px;\n    height: 300px;\n\n    VerticalBox {\n        a := Button { text: \"A\"; }\n        b := Button { text: \"B\"; }\n    }\n}\n";

fn parse(src: &str) -> perox_slint::Component {
    let d = perox_slint::parse(src);
    assert!(d.errors.is_empty(), "{src}\n{:?}", d.errors);
    d.components.into_iter().next().unwrap()
}

#[test]
fn the_canvas_shows_slint_and_knows_where_elements_are() {
    let h = Harness::new(FORM);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
    let w = h.rect(&[]);
    assert_eq!(w.size(), vec2(400.0, 300.0));
    let (a, b) = (h.rect(&[0, 0]), h.rect(&[0, 1]));
    assert!(w.contains_rect(a) && w.contains_rect(b));
    assert!(a.bottom() <= b.top(), "a above b: {a:?} {b:?}");
}

#[test]
fn drag_from_the_palette_inserts_between_widgets() {
    let mut h = Harness::new(FORM);
    let a = h.rect(&[0, 0]);
    let from = h.palette("Text").center();
    // Lower half of `a`: lands between a and b.
    h.drag(from, a.center() + vec2(0.0, a.height() * 0.3));
    let c = parse(&h.src);
    let kinds: Vec<_> = c
        .at(&[0])
        .unwrap()
        .children
        .iter()
        .map(|c| c.kind.as_str())
        .collect();
    assert_eq!(kinds, ["Button", "Text", "Button"], "{}", h.src);
    assert!(h.src.starts_with("// keep this comment\n"));
    assert_eq!(h.designer.selected, Some(vec![0, 1]));
}

#[test]
fn dropping_a_std_widget_imports_it() {
    let mut h = Harness::new(FORM);
    let a = h.rect(&[0, 0]);
    let from = h.palette("LineEdit").center();
    h.drag(from, a.center() + vec2(0.0, a.height() * 0.3));
    assert!(
        h.src
            .contains("import { Button, VerticalBox, LineEdit } from"),
        "{}",
        h.src
    );
    assert!(h.src.contains("line_edit1 := LineEdit { }"), "{}", h.src);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
}

#[test]
fn click_selects_and_delete_removes() {
    let mut h = Harness::new(FORM);
    let b = h.rect(&[0, 1]);
    h.click(b.center());
    assert_eq!(h.designer.selected, Some(vec![0, 1]));
    h.key(egui::Key::Delete);
    assert!(!h.src.contains("b := Button"), "{}", h.src);
    assert!(h.src.contains("a := Button"));
    assert_eq!(h.designer.selected, Some(vec![0]));
}

#[test]
fn dragging_a_widget_moves_it() {
    let mut h = Harness::new(FORM);
    let a = h.rect(&[0, 0]);
    let b = h.rect(&[0, 1]);
    h.drag(a.center(), b.center() + vec2(0.0, b.height() * 0.4));
    let c = parse(&h.src);
    let ids: Vec<_> = c
        .at(&[0])
        .unwrap()
        .children
        .iter()
        .map(|c| c.id.clone().unwrap())
        .collect();
    assert_eq!(ids, ["b", "a"], "{}", h.src);
    assert_eq!(h.designer.selected, Some(vec![0, 1]));
}

#[test]
fn resizing_the_window_writes_its_size() {
    let mut h = Harness::new(FORM);
    let w = h.designer.layout.window.unwrap();
    let corner = w.right_bottom() - vec2(2.0, 2.0);
    h.drag(corner, corner + vec2(100.0, 50.0));
    assert!(
        h.src.contains("width: 500px;") && h.src.contains("height: 350px;"),
        "{}",
        h.src
    );
}

const FREE: &str = "import { Button } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    title: \"T\";\n    preferred-width: 400px;\n    preferred-height: 300px;\n\n    a := Button { text: \"A\"; x: 16px; y: 16px; }\n}\n";

#[test]
fn drop_into_a_free_window_places_at_the_pointer_snapped() {
    let mut h = Harness::new(FREE);
    let origin = h.rect(&[]).min;
    let from = h.palette("Text").center();
    h.drag(from, origin + vec2(101.0, 61.0));
    let c = parse(&h.src);
    let l = c.at(&[1]).unwrap();
    assert_eq!(l.kind, "Text", "{}", h.src);
    assert_eq!(
        (l.num_prop("x"), l.num_prop("y")),
        (Some(104.0), Some(64.0)),
        "{}",
        h.src
    );
}

#[test]
fn dragging_in_a_free_window_moves_by_the_drag() {
    let mut h = Harness::new(FREE);
    let a = h.rect(&[0]);
    assert_eq!(a.min - h.rect(&[]).min, vec2(16.0, 16.0));
    let from = a.center();
    h.drag(from, from + vec2(40.0, 24.0));
    let b = parse(&h.src).at(&[0]).unwrap().clone();
    assert_eq!(
        (b.num_prop("x"), b.num_prop("y")),
        (Some(56.0), Some(40.0)),
        "{}",
        h.src
    );
    assert_eq!(h.designer.selected, Some(vec![0]));
}

#[test]
fn resize_handles_write_geometry() {
    let mut h = Harness::new(FREE);
    let a = h.rect(&[0]);
    h.click(a.center());
    let corner = a.right_bottom();
    h.drag(corner, corner + vec2(50.0, 20.0));
    let b = parse(&h.src).at(&[0]).unwrap().clone();
    assert_eq!(
        (b.num_prop("x"), b.num_prop("y")),
        (Some(16.0), Some(16.0)),
        "{}",
        h.src
    );
    let w = b.num_prop("width").unwrap();
    assert!(
        w > f64::from(a.width()) + 40.0 && w % 8.0 == 0.0,
        "snapped right edge: {}",
        h.src
    );
    // And the canvas shows the new size.
    assert!((h.rect(&[0]).width() - w as f32).abs() < 1.0);
}

#[test]
fn arrow_keys_nudge() {
    let mut h = Harness::new(FREE);
    let a = h.rect(&[0]);
    h.click(a.center());
    h.key(egui::Key::ArrowRight);
    h.key(egui::Key::ArrowDown);
    assert!(
        h.src.contains("x: 17px") && h.src.contains("y: 17px"),
        "{}",
        h.src
    );
}

#[test]
fn break_layout_keeps_widgets_where_they_are() {
    let mut h = Harness::new(FORM);
    let b_before = h.rect(&[0, 1]);
    let rects = h.designer.layout.elements.clone();
    h.src = perox_designer::free::break_layout(
        &h.src,
        "Main",
        perox_designer::free::Drawn { rects: &rects },
        &[0],
    )
    .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.src.contains("Rectangle {"), "{}", h.src);
    let b_after = h.rect(&[0, 1]);
    assert!(
        (b_before.min - b_after.min).length() < 1.0,
        "{b_before:?} {b_after:?}\n{}",
        h.src
    );
    assert!((b_before.size() - b_after.size()).length() < 1.0);
}

#[test]
fn go_to_handler_declares_and_wires_a_callback() {
    let mut h = Harness::new(FORM);
    let actions = h.designer.go_to_handler(&h.src.clone(), &[0, 0], "clicked");
    let mut opened = None;
    for a in actions {
        match a {
            Action::Edit(e) => h.src = perox_slint::edit::apply(&h.src, &e),
            Action::OpenHandler {
                form,
                method,
                params,
            } => opened = Some((form, method, params)),
            _ => {}
        }
    }
    assert_eq!(opened, Some(("Main".into(), "a_clicked".into(), vec![])));
    assert!(
        h.src.contains(
            "    callback a_clicked();
"
        ),
        "{}",
        h.src
    );
    assert!(
        h.src
            .contains("a := Button { text: \"A\"; clicked => { root.a_clicked(); } }"),
        "{}",
        h.src
    );
    // The second time it just opens it.
    let again = h.designer.go_to_handler(&h.src.clone(), &[0, 0], "clicked");
    assert!(
        matches!(again.as_slice(), [Action::OpenHandler { .. }]),
        "{again:?}"
    );
    // With parameters.
    let src = "import { LineEdit } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    name := LineEdit { }\n}\n";
    let mut h = Harness::new(src);
    let actions = h.designer.go_to_handler(src, &[0], "edited");
    let Some(Action::OpenHandler { params, .. }) = actions.last() else {
        panic!("{actions:?}")
    };
    assert_eq!(
        params,
        &[("text".to_owned(), "slint::SharedString".to_owned())]
    );
    for a in &actions {
        if let Action::Edit(e) = a {
            h.src = perox_slint::edit::apply(&h.src, e);
        }
    }
    assert!(
        h.src.contains("callback name_edited(text: string);"),
        "{}",
        h.src
    );
    assert!(
        h.src
            .contains("edited(text) => { root.name_edited(text); }"),
        "{}",
        h.src
    );
    h.frame(vec![]);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
}

#[test]
fn exposing_a_property_names_the_element_and_compiles() {
    let src = "import { LineEdit, VerticalBox } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    VerticalBox {\n        LineEdit { }\n    }\n}\n";
    let mut h = Harness::new(src);
    h.src = perox_designer::expose_property(&h.src, "Main", &[0, 0], "text").unwrap();
    assert!(h.src.contains("line_edit1 := LineEdit { }"), "{}", h.src);
    assert!(
        h.src
            .contains("in-out property <string> line_edit1_text <=> line_edit1.text;"),
        "{}",
        h.src
    );
    h.frame(vec![]);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
    // Colours are brushes; booleans and lengths work too.
    for (prop, ty) in [("enabled", "bool"), ("width", "length")] {
        h.src = perox_designer::expose_property(&h.src, "Main", &[0, 0], prop).unwrap();
        assert!(
            h.src.contains(&format!("<{ty}> line_edit1_{prop} <=>")),
            "{}",
            h.src
        );
    }
    h.frame(vec![]);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
}

#[test]
fn the_preview_follows_the_dark_switch() {
    for style in ["fluent", "cupertino"] {
        let mut h = Harness::new(FORM);
        h.designer.style = Some(style.into());
        let lum = |h: &Harness| {
            let p = h.designer.preview.pixel(390, 290).expect("rendered");
            p[0] as u32 + p[1] as u32 + p[2] as u32
        };
        h.designer.dark = Some(false);
        h.frame(vec![]);
        h.frame(vec![]);
        let light = lum(&h);
        h.designer.dark = Some(true);
        h.frame(vec![]);
        h.frame(vec![]);
        let dark = lum(&h);
        assert!(
            light > 500 && dark < 200,
            "{style}: light {light}, dark {dark}"
        );
    }
}

const THREE: &str = "import { Button } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    preferred-width: 400px;\n    preferred-height: 300px;\n    a := Button { text: \"A\"; x: 16px; y: 16px; }\n    b := Button { text: \"B\"; x: 120px; y: 64px; }\n    c := Button { text: \"C\"; x: 240px; y: 40px; }\n}\n";

impl Harness {
    fn shift_click(&mut self, at: Pos2) {
        // egui reads held keys from ModifiersChanged, as a real window sends it.
        let m = Modifiers::SHIFT;
        self.frame(vec![Event::ModifiersChanged(m), Event::PointerMoved(at)]);
        self.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: m,
        }]);
        self.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: m,
        }]);
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }
}

#[test]
fn shift_click_selects_several_and_they_align() {
    let mut h = Harness::new(THREE);
    let (a, b, c) = (h.rect(&[0]), h.rect(&[1]), h.rect(&[2]));
    h.click(a.center());
    h.shift_click(b.center());
    h.shift_click(c.center());
    assert_eq!(h.designer.selection(), vec![vec![2], vec![1], vec![0]]);
    // Shift-click again takes one out.
    h.shift_click(b.center());
    assert_eq!(h.designer.selection(), vec![vec![2], vec![0]]);
    let rects = h.designer.layout.elements.clone();
    h.src = perox_designer::free::arrange(
        &h.src,
        "Main",
        perox_designer::free::Drawn { rects: &rects },
        &h.designer.selection(),
        perox_designer::free::Arrange::Top,
    )
    .unwrap();
    assert!(
        h.src
            .contains("a := Button { text: \"A\"; x: 16px; y: 40px; }"),
        "{}",
        h.src
    );
    // Deleting removes all of them.
    h.frame(vec![]);
    h.key(egui::Key::Delete);
    assert!(
        !h.src.contains("a := ") && !h.src.contains("c := ") && h.src.contains("b := "),
        "{}",
        h.src
    );
}

#[test]
fn copy_and_paste_elements() {
    let mut h = Harness::new(THREE);
    let a = h.rect(&[0]);
    h.click(a.center());
    h.frame(vec![Event::Copy]);
    h.frame(vec![Event::Paste(
        "a := Button { text: \"A\"; x: 16px; y: 16px; }".into(),
    )]);
    assert!(
        h.src
            .contains("a1 := Button { text: \"A\"; x: 32px; y: 32px; }"),
        "{}",
        h.src
    );
    assert_eq!(h.designer.selected, Some(vec![1]));
    // Not Slint: ignored.
    let before = h.src.clone();
    h.frame(vec![Event::Paste("hello world".into())]);
    assert_eq!(h.src, before);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
}

#[test]
fn non_visual_elements_go_to_the_root_and_show_in_the_tray() {
    let mut h = Harness::new(FORM);
    assert!(
        h.designer.layout.tray.is_empty(),
        "no tray without components"
    );
    let from = h.palette("Timer").center();
    assert!(from.y < 1200.0, "Timer is visible in the palette: {from:?}");
    // Dropped on a button inside the layout: still goes to the root, before the layout.
    let to = h.rect(&[0, 1]).center();
    h.drag(from, to);
    let c = parse(&h.src);
    assert_eq!(c.root.children[0].kind, "Timer", "{}", h.src);
    assert_eq!(c.root.children[0].id.as_deref(), Some("timer1"));
    assert!(
        h.src.contains("timer1 := Timer { interval: 1s; }"),
        "{}",
        h.src
    );
    assert_eq!(c.root.children[1].kind, "VerticalBox");
    // Not on the canvas, but in the tray, and clicking it there selects it.
    h.frame(vec![]);
    assert!(
        h.designer
            .layout
            .elements
            .iter()
            .all(|(p, _)| p != &vec![0])
    );
    let chip = h
        .designer
        .layout
        .tray
        .iter()
        .find(|(p, _)| p == &vec![0])
        .expect("in the tray")
        .1;
    h.designer.selected = None;
    h.click(chip.center());
    assert_eq!(h.designer.selected, Some(vec![0]));

    // The menu bar goes first; a second one is refused.
    let from = h.palette("MenuBar").center();
    let to = h.rect(&[1]).center();
    h.drag(from, to);
    let c = parse(&h.src);
    assert_eq!(c.root.children[0].kind, "MenuBar", "{}", h.src);
    assert_eq!(c.root.children[1].kind, "Timer");
    let before = h.src.clone();
    let from = h.palette("MenuBar").center();
    let to = h.rect(&[2]).center();
    h.drag(from, to);
    assert_eq!(h.src, before);
    assert_eq!(
        h.designer.error.as_deref(),
        Some("This window already has a menu bar")
    );
    // Layout elements are still hit where they were drawn (paths moved by two).
    assert!(
        h.designer
            .layout
            .elements
            .iter()
            .any(|(p, _)| p == &vec![2, 1])
    );
}

#[test]
fn menus_preview_and_take_no_canvas_space() {
    let src = "import { VerticalBox, Button } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    width: 300px;\n    height: 200px;\n    MenuBar {\n        Menu {\n            title: \"File\";\n            open := MenuItem { title: \"Open\"; shortcut: @keys(Control + O); }\n        }\n    }\n    VerticalBox { b := Button { text: \"B\"; } }\n}\n";
    let h = Harness::new(src);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
    let paths: Vec<_> = h
        .designer
        .layout
        .elements
        .iter()
        .map(|(p, _)| p.clone())
        .collect();
    assert!(
        !paths.iter().any(|p| p.first() == Some(&0) && !p.is_empty()),
        "{paths:?}"
    );
    // The content sits under Slint's menu bar.
    let window = h.rect(&[]);
    assert!(
        h.rect(&[1]).top() > window.top() + 10.0,
        "{:?} {:?}",
        h.rect(&[1]),
        window
    );
}

/// A 2×2 red PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a,
    0x73, 0x00, 0x00, 0x00, 0x12, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x44, 0x0c, 0x20, 0x02, 0x00, 0x2a, 0xf2, 0x05, 0xfb, 0x86, 0xbc, 0x11, 0x5b, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn project_images_drag_onto_the_form_as_image_elements() {
    let dir = std::env::temp_dir().join(format!("perox-images-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("images")).unwrap();
    std::fs::write(dir.join("images/logo.png"), PNG).unwrap();
    let mut h = Harness::at(FORM, &dir.join("main.slint"));
    let row = h
        .designer
        .layout
        .images
        .iter()
        .find(|(rel, _)| rel == "images/logo.png")
        .expect("the image is in the palette")
        .1;
    let to = h.rect(&[0, 1]).center();
    h.drag(row.center(), to);
    assert!(
        h.src
            .contains("Image { source: @image-url(\"images/logo.png\"); }"),
        "{}",
        h.src
    );
    h.frame(vec![]);
    assert!(
        h.designer.preview.diagnostics.is_empty(),
        "{:?}",
        h.designer.preview.diagnostics
    );
    // A later plain Image from the palette doesn't inherit the source.
    let from = h.palette("Image").center();
    h.drag(from, to);
    assert_eq!(h.src.matches("@image-url").count(), 1, "{}", h.src);
}
