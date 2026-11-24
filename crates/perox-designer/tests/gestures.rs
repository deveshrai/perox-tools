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
}

impl Harness {
    fn new(src: &str) -> Self {
        let mut h = Self {
            ctx: egui::Context::default(),
            designer: Designer::new(),
            src: src.to_owned(),
            opened: Vec::new(),
        };
        h.frame(vec![]);
        h.frame(vec![]);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
            events,
            ..Default::default()
        };
        let mut actions = Vec::new();
        let (designer, src) = (&mut self.designer, &self.src);
        let mut out = self.ctx.run_ui(input, |ui| {
            actions = designer.show(ui, src, std::path::Path::new("main.slint"));
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
