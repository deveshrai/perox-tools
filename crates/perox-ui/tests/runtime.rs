//! Drives forms headlessly through egui with synthetic input.

use perox_ui::egui::{self, Event, PointerButton, Pos2, Rect, vec2};
use perox_ui::{DesignOutput, Form, FormEvent};

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(600.0, 400.0));

struct Harness {
    ctx: egui::Context,
    form: Form,
}

impl Harness {
    fn new(src: &str) -> Self {
        let h = Self {
            ctx: egui::Context::default(),
            form: Form::new(src),
        };
        assert!(
            h.form.file().errors.is_empty(),
            "{:?}",
            h.form.file().errors
        );
        h
    }

    fn input(events: Vec<Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        }
    }

    fn frame(&mut self, events: Vec<Event>) -> Vec<FormEvent> {
        let mut out = Vec::new();
        let form = &mut self.form;
        let mut output = self.ctx.run_ui(Self::input(events), |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| out = form.show(ui));
        });
        output.textures_delta.clear();
        out
    }

    fn design(&mut self, events: Vec<Event>, selected: Option<&[usize]>) -> DesignOutput {
        let mut out = DesignOutput::default();
        let form = &mut self.form;
        let mut output = self.ctx.run_ui(Self::input(events), |ui| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| out = form.show_design(ui, selected));
        });
        output.textures_delta.clear();
        out
    }

    fn rect(&mut self, id: &str) -> Rect {
        let path = self.form.file().find(id).expect("no such id");
        let out = self.design(vec![], None);
        out.rects
            .into_iter()
            .find(|(p, _)| *p == path)
            .map(|(_, r)| r)
            .expect("not laid out")
    }

    fn click(&mut self, at: Pos2) -> Vec<FormEvent> {
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        let mut all = self.frame(vec![Event::PointerMoved(at)]);
        all.extend(self.frame(vec![press(true)]));
        all.extend(self.frame(vec![press(false)]));
        all.extend(self.frame(vec![]));
        all
    }

    fn click_id(&mut self, id: &str) -> Vec<FormEvent> {
        self.frame(vec![]);
        let r = self.rect(id);
        self.click(r.left_center() + vec2(8.0, 0.0))
    }
}

#[test]
fn button_click_names_its_handler() {
    let mut h = Harness::new(
        "Window w { VBox { Button save { text: \"Save\" on clicked: save_clicked } } }",
    );
    let events = h.click_id("save");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].event, "clicked");
    assert_eq!(events[0].id.as_deref(), Some("save"));
    assert_eq!(events[0].handler.as_deref(), Some("save_clicked"));
}

#[test]
fn column_layout_stretches_width_and_spacer_pushes_down() {
    let mut h = Harness::new(
        "Window w {\n margin: 10\n VBox {\n Button a { text: \"A\" }\n Spacer {}\n Button b { text: \"B\" }\n }\n}",
    );
    h.frame(vec![]);
    let a = h.rect("a");
    let b = h.rect("b");
    assert!((a.width() - 580.0).abs() < 1.0, "{a:?}");
    assert!((a.top() - 10.0).abs() < 1.0, "{a:?}");
    assert!((b.bottom() - 390.0).abs() < 1.0, "{b:?}");
}

#[test]
fn row_layout_centres_fixed_height_widgets_and_grows_text_boxes() {
    let mut h = Harness::new(
        "Window w {\n margin: 0\n HBox {\n height: 100\n Button a { text: \"A\" }\n TextEdit t {}\n }\n}",
    );
    h.frame(vec![]);
    let a = h.rect("a");
    let t = h.rect("t");
    assert!((a.center().y - 50.0).abs() < 1.0, "{a:?}");
    assert!(t.right() > 595.0, "text box should take the rest: {t:?}");
}

#[test]
fn grid_fits_the_label_column_and_stretches_the_field() {
    let mut h = Harness::new(
        "Window w {\n margin: 0\n Grid {\n columns: [0, 1]\n Label l { text: \"Name\" }\n TextEdit f {}\n }\n}",
    );
    h.frame(vec![]);
    let l = h.rect("l");
    let f = h.rect("f");
    assert!(l.width() < 80.0, "{l:?}");
    assert!(f.left() > l.right() && f.right() > 595.0, "{f:?}");
}

#[test]
fn check_box_and_radio_groups() {
    let mut h = Harness::new(
        "Window w { VBox {\n CheckBox c { text: \"C\" on toggled: c_toggled }\n RadioButton r1 { text: \"One\"; checked: true }\n RadioButton r2 { text: \"Two\" }\n } }",
    );
    let e = h.click_id("c");
    assert_eq!(e[0].handler.as_deref(), Some("c_toggled"));
    assert!(h.form.widget("c").checked());
    h.click_id("r2");
    assert!(h.form.widget("r2").checked());
    assert!(
        !h.form.widget("r1").checked(),
        "radio buttons in one parent are exclusive"
    );
}

#[test]
fn typing_changes_text_and_enter_submits() {
    let mut h = Harness::new(
        "Window w { VBox { TextEdit name { on changed: edited; on submitted: go } } }",
    );
    h.click_id("name");
    let e = h.frame(vec![Event::Text("Ada".into())]);
    assert_eq!(e[0].handler.as_deref(), Some("edited"));
    assert_eq!(h.form.widget("name").text(), "Ada");
    let key = |pressed| Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    let mut e = h.frame(vec![key(true)]);
    e.extend(h.frame(vec![key(false)]));
    assert!(e.iter().any(|e| e.event == "submitted"), "{e:?}");
}

#[test]
fn runtime_values_override_the_file_and_hide_widgets() {
    let mut h = Harness::new(
        "Window w { VBox {\n Label status { text: \"idle\" }\n Button b { text: \"B\" }\n } }",
    );
    assert_eq!(h.form.widget("status").text(), "idle");
    h.form.widget("status").set_text("busy");
    assert_eq!(h.form.widget("status").text(), "busy");
    h.form.widget("b").set_visible(false);
    h.frame(vec![]);
    let out = h.design(vec![], None);
    assert!(!out.rects.iter().any(|(p, _)| *p == vec![0, 1]));
}

#[test]
fn tabs_switch_pages() {
    let mut h = Harness::new(
        "Window w { Tabs tabs {\n Tab { title: \"One\"\n Label a { text: \"A\" } }\n Tab { title: \"Two\"\n Label b { text: \"B\" } }\n } }",
    );
    h.frame(vec![]);
    let out = h.design(vec![], None);
    let tabs = out.rects.iter().find(|(p, _)| *p == vec![0]).unwrap().1;
    assert!(
        out.rects.iter().all(|(p, _)| *p != vec![0, 1, 0]),
        "page two is hidden"
    );
    // Second title sits right of the first; aim past the first one.
    let e = h.click(tabs.left_top() + vec2(60.0, 10.0));
    assert_eq!(e.first().map(|e| e.event), Some("changed"), "{e:?}");
    assert_eq!(h.form.widget("tabs").selected(), Some(1));
    let out = h.design(vec![], None);
    assert!(out.rects.iter().any(|(p, _)| *p == vec![0, 1, 0]));
}

#[test]
fn designer_selects_the_deepest_element_and_widgets_stay_inert() {
    let mut h = Harness::new(
        "Window w { VBox {\n GroupBox { title: \"G\"\n Button b { text: \"B\" on clicked: hit } }\n } }",
    );
    h.frame(vec![]);
    let b = h.rect("b");
    let at = b.center();
    let press = |pressed| Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    h.design(vec![Event::PointerMoved(at)], None);
    h.design(vec![press(true)], None);
    let out = h.design(vec![press(false)], None);
    assert_eq!(out.clicked, Some(vec![0, 0, 0]));
    assert_eq!(out.hovered, Some(vec![0, 0, 0]));
}

#[test]
fn free_containers_place_children_at_x_y() {
    let mut h = Harness::new(
        "Window w {\n layout: free\n Button a { text: \"A\"; x: 20; y: 30; width: 100 }\n Label b { text: \"B\"; x: 200; y: 8 }\n}",
    );
    h.frame(vec![]);
    let a = h.rect("a");
    assert_eq!(a.min, Pos2::new(20.0, 30.0), "{a:?}");
    assert!((a.width() - 100.0).abs() < 0.5);
    let b = h.rect("b");
    assert_eq!(b.min, Pos2::new(200.0, 8.0));
    // Clicks still work on freely placed widgets.
    let e = h.click(a.center());
    assert_eq!(e.first().map(|e| e.event), Some("clicked"));
}

#[test]
fn a_panel_in_a_layout_is_as_big_as_its_children() {
    let mut h = Harness::new(
        "Window w {\n margin: 0\n VBox {\n Panel p {\n Button a { x: 10; y: 50; width: 80; height: 30 }\n }\n Button after {}\n }\n}",
    );
    h.frame(vec![]);
    let p = h.rect("p");
    let a = h.rect("a");
    assert_eq!(a.min, p.min + vec2(10.0, 50.0));
    let after = h.rect("after");
    assert!(
        after.top() >= p.top() + 80.0,
        "panel reserves its children's height: {p:?} {after:?}"
    );
}

#[test]
fn free_group_box_offsets_children_below_its_title() {
    let mut h = Harness::new(
        "Window w {\n layout: free\n GroupBox g { title: \"G\"; layout: free; x: 0; y: 0; width: 200; height: 120\n Button a { x: 0; y: 0 }\n }\n}",
    );
    h.frame(vec![]);
    let g = h.rect("g");
    let a = h.rect("a");
    assert!(a.top() > g.top() + 10.0, "below the title: {g:?} {a:?}");
    assert!(a.left() > g.left(), "inside the frame");
}
