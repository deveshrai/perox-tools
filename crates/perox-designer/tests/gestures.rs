//! The designer driven headlessly with synthetic mouse and keyboard input. Each
//! gesture must turn into a minimal edit of the `.pxui` text.

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use perox_designer::{Action, Designer};

struct Harness {
    ctx: egui::Context,
    designer: Designer,
    src: String,
    opened: Vec<(String, String)>,
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
            actions = designer.show(ui, src, "main");
        });
        out.textures_delta.clear();
        for a in actions {
            match a {
                Action::Edit(edits) => self.src = perox_forms::edit::apply(&self.src, &edits),
                Action::OpenHandler { form, method } => self.opened.push((form, method)),
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

const FORM: &str = "// keep this comment\nWindow main {\n    title: \"T\"\n    size: 400 x 300\n\n    VBox {\n        Button a { text: \"A\" }\n        Button b { text: \"B\" }\n    }\n}\n";

#[test]
fn drag_from_the_palette_inserts_between_widgets() {
    let mut h = Harness::new(FORM);
    let a = h.rect(&[0, 0]);
    let from = h.palette("Label").center();
    // Lower half of `a`: lands between a and b.
    h.drag(from, a.center() + vec2(0.0, a.height() * 0.3));
    let f = perox_forms::parse(&h.src);
    assert!(f.errors.is_empty(), "{}", h.src);
    let vbox = f.at(&[0]).unwrap();
    let kinds: Vec<_> = vbox.children.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["Button", "Label", "Button"], "{}", h.src);
    assert!(h.src.starts_with("// keep this comment\n"));
    assert_eq!(h.designer.selected, Some(vec![0, 1]));
}

#[test]
fn click_selects_and_delete_removes() {
    let mut h = Harness::new(FORM);
    let b = h.rect(&[0, 1]);
    h.click(b.center());
    assert_eq!(h.designer.selected, Some(vec![0, 1]));
    h.key(egui::Key::Delete);
    assert!(!h.src.contains("Button b"), "{}", h.src);
    assert!(h.src.contains("Button a"));
    assert_eq!(h.designer.selected, Some(vec![0]));
}

#[test]
fn dragging_a_widget_moves_it() {
    let mut h = Harness::new(FORM);
    let a = h.rect(&[0, 0]);
    let b = h.rect(&[0, 1]);
    h.drag(a.center(), b.center() + vec2(0.0, b.height() * 0.4));
    let f = perox_forms::parse(&h.src);
    let ids: Vec<_> = f
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
    assert!(h.src.contains("size: 500 x 350"), "{}", h.src);
}

const FREE: &str = "Window main {\n    title: \"T\"\n    size: 400 x 300\n    layout: free\n\n    Button a { text: \"A\"; x: 16; y: 16 }\n}\n";

fn content_origin(h: &Harness, path: &[usize]) -> Pos2 {
    let i = h
        .designer
        .layout
        .elements
        .iter()
        .position(|(p, _)| p == path)
        .unwrap();
    h.designer.layout.contents[i].min
}

#[test]
fn drop_into_a_free_window_places_at_the_pointer_snapped() {
    let mut h = Harness::new(FREE);
    let origin = content_origin(&h, &[]);
    let from = h.palette("Label").center();
    h.drag(from, origin + vec2(101.0, 61.0));
    let f = perox_forms::parse(&h.src);
    let l = f.at(&[1]).unwrap();
    assert_eq!(l.kind, "Label", "{}", h.src);
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
    let from = a.center();
    h.drag(from, from + vec2(40.0, 24.0));
    let f = perox_forms::parse(&h.src);
    let b = f.at(&[0]).unwrap();
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
    let f = perox_forms::parse(&h.src);
    let b = f.at(&[0]).unwrap();
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
}

#[test]
fn arrow_keys_nudge() {
    let mut h = Harness::new(FREE);
    let a = h.rect(&[0]);
    h.click(a.center());
    h.key(egui::Key::ArrowRight);
    h.key(egui::Key::ArrowDown);
    assert!(
        h.src.contains("x: 17") && h.src.contains("y: 17"),
        "{}",
        h.src
    );
}

#[test]
fn break_layout_keeps_widgets_where_they_are() {
    let mut h = Harness::new(FORM);
    let b_before = h.rect(&[0, 1]);
    let rects = h.designer.layout.elements.clone();
    let contents = h.designer.layout.contents.clone();
    h.src = perox_designer::free::break_layout(
        &h.src,
        perox_designer::free::Drawn {
            rects: &rects,
            contents: &contents,
        },
        &[0],
    )
    .unwrap();
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(h.src.contains("Panel"), "{}", h.src);
    let b_after = h.rect(&[0, 1]);
    assert!(
        (b_before.min - b_after.min).length() < 1.0,
        "{b_before:?} {b_after:?}\n{}",
        h.src
    );
}
