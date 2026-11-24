//! Graphing Calculator: a scientific calculator, a 2D graph of up to three functions of
//! x, and a 3D surface z = f(x, y). The window is `ui/main_window.slint`; this is its
//! code-behind.

mod expr;
mod plot2d;
mod plot3d;

perox_ui::include_ui!();

use std::rc::Rc;

use perox_ui::canvas::Canvas;
use slint::{ComponentHandle, ModelRc, SharedString, StandardListViewItem, VecModel};

const HINT_2D: &str = "Drag to pan · scroll to zoom · double-click to reset";
const HINT_3D: &str = "Drag to turn · scroll to zoom · double-click to reset";

struct App {
    /// The last result, for `ans`.
    ans: f64,
    degrees: bool,
    /// A result is showing: the next digit starts afresh, the next operator works on it.
    fresh: bool,
    /// Earlier calculations, newest first: (expression, result).
    history: Vec<(String, String)>,
    mode: i32,
    view2d: plot2d::View,
    view3d: plot3d::View,
    /// Where the mouse was at the last press or move, in points.
    last: (f32, f32),
}

impl Default for App {
    fn default() -> Self {
        Self {
            ans: 0.0,
            degrees: true,
            fresh: false,
            history: Vec::new(),
            mode: 0,
            view2d: plot2d::View::default(),
            view3d: plot3d::View::default(),
            last: (0.0, 0.0),
        }
    }
}

/// What a key adds to the expression.
fn key_text(label: &str) -> String {
    match label {
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "ln" | "log" => format!("{label}("),
        "√" => "√(".into(),
        "x²" => "²".into(),
        "n!" => "!".into(),
        "EXP" => "×10^".into(),
        "Ans" => "ans".into(),
        other => other.into(),
    }
}

/// Keys that work on what's already there (so after a result they continue from it).
fn continues(text: &str) -> bool {
    matches!(text, "+" | "−" | "×" | "÷" | "^" | "²" | "!" | "%")
}

impl App {
    fn redraw(&mut self, ui: &MainWindow) {
        let (w, h) = (ui.get_plot_width(), ui.get_plot_height());
        if w < 2.0 || h < 2.0 {
            return;
        }
        let mut canvas = Canvas::new(ui.window(), w, h);
        let mut errors = [false; 4];
        let labels = if self.mode == 0 {
            let texts = [ui.get_fn1(), ui.get_fn2(), ui.get_fn3()];
            let funcs: Vec<_> = texts
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let t = t.trim();
                    let e = if t.is_empty() {
                        None
                    } else {
                        let parsed = expr::parse(t).ok();
                        errors[i] = parsed.is_none();
                        parsed
                    };
                    (e, plot2d::COLORS[i])
                })
                .collect();
            plot2d::draw(&mut canvas.pixmap, canvas.scale, &self.view2d, &funcs)
        } else {
            match expr::parse(&ui.get_fn_z()) {
                Ok(e) => plot3d::draw(&mut canvas.pixmap, canvas.scale, &self.view3d, &e),
                Err(_) => {
                    errors[3] = true;
                    Vec::new()
                }
            }
        };
        ui.set_plot_image(canvas.image());
        let labels: Vec<PlotLabel> = labels
            .into_iter()
            .map(|l| PlotLabel {
                x: l.x,
                y: l.y,
                text: l.text.into(),
            })
            .collect();
        ui.set_plot_labels(ModelRc::new(VecModel::from(labels)));
        ui.set_fn_errors(ModelRc::new(VecModel::from(errors.to_vec())));
    }

    fn show_history(&self, ui: &MainWindow) {
        let items: Vec<StandardListViewItem> = self
            .history
            .iter()
            .map(|(e, r)| StandardListViewItem::from(SharedString::from(format!("{e} = {r}"))))
            .collect();
        ui.set_history(ModelRc::new(VecModel::from(items)));
    }

    fn set_mode(&mut self, ui: &MainWindow, mode: i32) {
        self.mode = mode;
        ui.set_mode(mode);
        ui.set_plot_hint(if mode == 0 { HINT_2D } else { HINT_3D }.into());
        self.redraw(ui);
    }
}

impl MainWindowHandlers for App {
    fn key_pressed(&mut self, ui: &MainWindow, label: SharedString) {
        let mut text = ui.get_expression().to_string();
        match label.as_str() {
            "AC" => {
                ui.set_expression(SharedString::new());
                ui.set_result("0".into());
                ui.set_result_error(false);
                self.fresh = false;
                return;
            }
            "⌫" => {
                text.pop();
                self.fresh = false;
            }
            "=" => return self.evaluate(ui),
            "Plot" => return self.plot_expression(ui),
            _ => {
                let add = key_text(&label);
                if self.fresh {
                    text = if continues(&add) {
                        "ans".into()
                    } else {
                        String::new()
                    };
                    self.fresh = false;
                }
                text.push_str(&add);
            }
        }
        ui.set_expression(text.into());
    }

    fn evaluate(&mut self, ui: &MainWindow) {
        let text = ui.get_expression().trim().to_string();
        let parsed = expr::parse(&text);
        match parsed {
            Ok(e) => {
                if e.uses_x() || e.uses_y() {
                    ui.set_result("x and y are for graphs: press Plot".into());
                    ui.set_result_error(true);
                    return;
                }
                let v = e.eval(&expr::Env {
                    ans: self.ans,
                    degrees: self.degrees,
                    ..Default::default()
                });
                let shown = expr::format(v);
                ui.set_result(shown.clone().into());
                ui.set_result_error(!v.is_finite());
                if v.is_finite() {
                    self.ans = v;
                    ui.set_memo(format!("Ans = {shown}").into());
                }
                self.history.insert(0, (text, shown));
                self.history.truncate(50);
                self.show_history(ui);
                self.fresh = true;
            }
            Err(err) => {
                ui.set_result(err.0.into());
                ui.set_result_error(true);
            }
        }
    }

    fn angle_picked(&mut self, ui: &MainWindow, index: i32) {
        self.degrees = index == 0;
        if self.fresh {
            self.evaluate(ui);
        }
    }

    fn history_picked(&mut self, ui: &MainWindow, index: i32) {
        if let Some((e, _)) = self.history.get(index as usize) {
            ui.set_expression(e.as_str().into());
            self.fresh = false;
        }
    }

    fn mode_picked(&mut self, ui: &MainWindow, index: i32) {
        self.set_mode(ui, index);
    }

    fn functions_edited(&mut self, ui: &MainWindow) {
        self.redraw(ui);
    }

    fn reset_view(&mut self, ui: &MainWindow) {
        self.view2d = plot2d::View::default();
        self.view3d = plot3d::View::default();
        self.redraw(ui);
    }

    fn plot_pressed(&mut self, _ui: &MainWindow, x: f32, y: f32) {
        self.last = (x, y);
    }

    fn plot_dragged(&mut self, ui: &MainWindow, x: f32, y: f32) {
        let (dx, dy) = ((x - self.last.0) as f64, (y - self.last.1) as f64);
        self.last = (x, y);
        if self.mode == 0 {
            self.view2d.cx -= dx / self.view2d.scale;
            self.view2d.cy += dy / self.view2d.scale;
        } else {
            self.view3d.drag(dx, dy);
        }
        self.redraw(ui);
    }

    fn plot_scrolled(&mut self, ui: &MainWindow, delta: f32) {
        let factor = 1.0015f64.powf(delta as f64);
        if self.mode == 0 {
            let (w, h) = (ui.get_plot_width(), ui.get_plot_height());
            let (mx, my) = self.last;
            self.view2d.zoom_at(w, h, mx, my, factor);
        } else {
            self.view3d.zoom = (self.view3d.zoom * factor).clamp(0.4, 3.0);
        }
        self.redraw(ui);
    }

    fn plot_resized(&mut self, ui: &MainWindow) {
        self.redraw(ui);
    }

    fn about_clicked(&mut self, ui: &MainWindow) {
        if let Ok(about) = AboutDialog::new() {
            let _ = about.exec(Some(ui.window()), |_| {});
        }
    }
}

impl App {
    /// Plot: the expression becomes a curve (or, with y in it, the surface).
    fn plot_expression(&mut self, ui: &MainWindow) {
        let text = ui.get_expression().trim().to_string();
        match expr::parse(&text) {
            Ok(e) => {
                if e.uses_y() {
                    ui.set_fn_z(text.into());
                    self.set_mode(ui, 1);
                } else {
                    ui.set_fn1(text.into());
                    self.set_mode(ui, 0);
                }
            }
            Err(err) => {
                ui.set_result(err.0.into());
                ui.set_result_error(true);
            }
        }
    }
}

/// Start-up options, for trying things out and for screenshots:
/// `--calc <expression>` (repeatable) works it out; `--3d [<z = f(x, y)>]` starts on
/// the surface.
fn apply_args(app: &mut App, ui: &MainWindow, args: &[String]) {
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--calc" => {
                if let Some(e) = it.next() {
                    ui.set_expression(e.as_str().into());
                    app.evaluate(ui);
                }
            }
            "--3d" => {
                if let Some(f) = it.next_if(|f| !f.starts_with("--")) {
                    ui.set_fn_z(f.as_str().into());
                }
                app.set_mode(ui, 1);
            }
            _ => {}
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_history(ModelRc::new(VecModel::<StandardListViewItem>::default()));
    let app = ui.connect(App::default());
    // The first picture, once the window has its size.
    let weak = ui.as_weak();
    let first = Rc::clone(&app);
    let args: Vec<String> = std::env::args().skip(1).collect();
    slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
        if let Some(ui) = weak.upgrade() {
            let mut app = first.borrow_mut();
            apply_args(&mut app, &ui, &args);
            app.redraw(&ui);
        }
    });
    ui.run()
}
