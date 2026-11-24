//! The 2D graph: a grid, the axes, and each function's curve, drawn with tiny-skia.

use perox_ui::canvas::tiny_skia::{
    Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::expr::{Env, Expr};

/// What part of the plane is shown: the point at the centre and points per unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub cx: f64,
    pub cy: f64,
    pub scale: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            cx: 0.0,
            cy: 0.0,
            scale: 60.0,
        }
    }
}

/// A label to show over the plot, in points.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub x: f32,
    pub y: f32,
    pub text: String,
}

/// A function to draw (if it parsed) and its colour.
pub type Curve = (Option<Expr>, (u8, u8, u8));

/// Curve colours, as the dots by the function fields.
pub const COLORS: [(u8, u8, u8); 3] = [(0x4c, 0x8d, 0xff), (0xff, 0x9f, 0x43), (0x2e, 0xd4, 0x7a)];

impl View {
    pub fn to_screen(self, w: f32, h: f32, x: f64, y: f64) -> (f32, f32) {
        (
            (w as f64 / 2.0 + (x - self.cx) * self.scale) as f32,
            (h as f64 / 2.0 - (y - self.cy) * self.scale) as f32,
        )
    }

    pub fn to_plane(self, w: f32, h: f32, sx: f32, sy: f32) -> (f64, f64) {
        (
            self.cx + (sx as f64 - w as f64 / 2.0) / self.scale,
            self.cy - (sy as f64 - h as f64 / 2.0) / self.scale,
        )
    }

    /// Zooms by `factor` keeping the plane point under (`sx`, `sy`) where it is.
    pub fn zoom_at(&mut self, w: f32, h: f32, sx: f32, sy: f32, factor: f64) {
        let (px, py) = self.to_plane(w, h, sx, sy);
        self.scale = (self.scale * factor).clamp(0.5, 50_000.0);
        let (qx, qy) = self.to_plane(w, h, sx, sy);
        self.cx += px - qx;
        self.cy += py - qy;
    }
}

/// A tidy grid step (1, 2 or 5 × 10ⁿ) at least `min` units apart.
pub fn nice_step(min: f64) -> f64 {
    let p = 10f64.powf(min.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if m * p >= min {
            return m * p;
        }
    }
    10.0 * p
}

/// A tick value without float noise: 0.30000000000000004 → 0.3.
pub fn tick_text(v: f64, step: f64) -> String {
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    let s = format!("{v:.decimals$}");
    if s == "-0" || s.starts_with("-0.") && s.trim_start_matches("-0.").chars().all(|c| c == '0') {
        "0".into()
    } else {
        s.replace('-', "−")
    }
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    }
}

fn paint(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(r, g, b, a));
    p.anti_alias = true;
    p
}

/// Draws the grid, axes and `funcs` (with their colours) on `pm`, `w` × `h` points at
/// `scale` pixels per point. Returns the axis labels to show over it.
pub fn draw(pm: &mut Pixmap, scale: f32, view: &View, funcs: &[Curve]) -> Vec<Label> {
    let (w, h) = (pm.width() as f32 / scale, pm.height() as f32 / scale);
    let t = Transform::from_scale(scale, scale);
    let mut labels = Vec::new();

    // Grid: minor lines every step, major every 5, at least ~18 points apart.
    let step = nice_step(18.0 / view.scale);
    let major = nice_step(80.0 / view.scale);
    let (x0, y1) = view.to_plane(w, h, 0.0, 0.0);
    let (x1, y0) = view.to_plane(w, h, w, h);
    let mut minor = PathBuilder::new();
    let mut majors = PathBuilder::new();
    let mut k = (x0 / step).floor() as i64;
    while (k as f64) * step <= x1 {
        let x = k as f64 * step;
        let (sx, _) = view.to_screen(w, h, x, 0.0);
        let is_major = ((x / major).round() * major - x).abs() < step * 0.01;
        let pb = if is_major { &mut majors } else { &mut minor };
        pb.move_to(sx, 0.0);
        pb.line_to(sx, h);
        k += 1;
    }
    let mut k = (y0 / step).floor() as i64;
    while (k as f64) * step <= y1 {
        let y = k as f64 * step;
        let (_, sy) = view.to_screen(w, h, 0.0, y);
        let is_major = ((y / major).round() * major - y).abs() < step * 0.01;
        let pb = if is_major { &mut majors } else { &mut minor };
        pb.move_to(0.0, sy);
        pb.line_to(w, sy);
        k += 1;
    }
    if let Some(p) = minor.finish() {
        pm.stroke_path(&p, &paint(255, 255, 255, 10), &stroke(1.0), t, None);
    }
    if let Some(p) = majors.finish() {
        pm.stroke_path(&p, &paint(255, 255, 255, 24), &stroke(1.0), t, None);
    }

    // Axes, and the labels along them (kept on screen when an axis isn't).
    let (ax, ay) = view.to_screen(w, h, 0.0, 0.0);
    let mut axes = PathBuilder::new();
    axes.move_to(ax, 0.0);
    axes.line_to(ax, h);
    axes.move_to(0.0, ay);
    axes.line_to(w, ay);
    if let Some(p) = axes.finish() {
        pm.stroke_path(&p, &paint(170, 180, 200, 110), &stroke(1.5), t, None);
    }
    let label_y = ay.clamp(4.0, h - 18.0);
    let mut k = (x0 / major).floor() as i64;
    while (k as f64) * major <= x1 {
        let x = k as f64 * major;
        let (sx, _) = view.to_screen(w, h, x, 0.0);
        if x.abs() > major * 0.01 && sx > 14.0 && sx < w - 24.0 {
            labels.push(Label {
                x: sx + 4.0,
                y: label_y + 2.0,
                text: tick_text(x, major),
            });
        }
        k += 1;
    }
    let label_x = (ax + 5.0).clamp(4.0, w - 40.0);
    let mut k = (y0 / major).floor() as i64;
    while (k as f64) * major <= y1 {
        let y = k as f64 * major;
        let (_, sy) = view.to_screen(w, h, 0.0, y);
        if y.abs() > major * 0.01 && sy > 10.0 && sy < h - 20.0 {
            labels.push(Label {
                x: label_x,
                y: sy - 16.0,
                text: tick_text(y, major),
            });
        }
        k += 1;
    }

    // The curves: one sample per half point; a gap where the function isn't defined
    // or jumps (tan's asymptotes).
    for (expr, (r, g, b)) in funcs {
        let Some(expr) = expr else { continue };
        let mut pb = PathBuilder::new();
        let mut last: Option<f32> = None;
        let samples = (w * 2.0) as usize;
        for i in 0..=samples {
            let sx = i as f32 / 2.0;
            let (x, _) = view.to_plane(w, h, sx, 0.0);
            let y = expr.eval(&Env {
                x,
                ..Default::default()
            });
            let sy = view.to_screen(w, h, 0.0, y).1;
            if !y.is_finite() || !(-10.0 * h..=11.0 * h).contains(&sy) {
                last = None;
                continue;
            }
            match last {
                Some(ly) if (sy - ly).abs() < h * 0.9 => pb.line_to(sx, sy),
                _ => pb.move_to(sx, sy),
            }
            last = Some(sy);
        }
        if let Some(p) = pb.finish() {
            // A soft glow under a crisp line.
            pm.stroke_path(&p, &paint(*r, *g, *b, 40), &stroke(7.0), t, None);
            pm.stroke_path(&p, &paint(*r, *g, *b, 255), &stroke(2.4), t, None);
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_and_ticks() {
        assert_eq!(nice_step(0.3), 0.5);
        assert_eq!(nice_step(1.2), 2.0);
        assert_eq!(nice_step(30.0), 50.0);
        assert_eq!(tick_text(0.30000000000000004, 0.1), "0.3");
        assert_eq!(tick_text(-2.0, 1.0), "−2");
        assert_eq!(tick_text(-0.0, 0.5), "0");
    }

    #[test]
    fn zoom_keeps_the_point_under_the_mouse() {
        let mut v = View::default();
        let before = v.to_plane(800.0, 600.0, 600.0, 100.0);
        v.zoom_at(800.0, 600.0, 600.0, 100.0, 2.0);
        let after = v.to_plane(800.0, 600.0, 600.0, 100.0);
        assert!((before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9);
        assert_eq!(v.scale, 120.0);
    }

    #[test]
    fn draws_a_curve_and_labels() {
        let mut pm = Pixmap::new(400, 300).unwrap();
        let f = crate::expr::parse("x").unwrap();
        let labels = draw(&mut pm, 1.0, &View::default(), &[(Some(f), COLORS[0])]);
        assert!(labels.iter().any(|l| l.text == "2"), "{labels:?}");
        // The line y = x passes through the middle, in blue.
        let p = pm.pixel(200, 150).unwrap();
        assert!(p.blue() > p.red(), "{p:?}");
    }
}
