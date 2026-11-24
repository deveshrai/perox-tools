//! The 3D graph: z = f(x, y) as a shaded surface in a box, turned with the mouse.
//!
//! The surface is a grid of quads, coloured by height, lit by a fixed light, and drawn
//! back to front (the painter's algorithm), with perspective.

use perox_ui::canvas::tiny_skia::{
    Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::expr::{Env, Expr};
use crate::plot2d::Label;

/// How the surface is seen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// Turn around the vertical axis, radians.
    pub yaw: f64,
    /// Tilt: 0 side on, π/2 from above.
    pub pitch: f64,
    pub zoom: f64,
    /// x and y run from −range to range.
    pub range: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            yaw: -0.75,
            pitch: 0.55,
            zoom: 1.0,
            range: 6.0,
        }
    }
}

impl View {
    /// Dragging by (`dx`, `dy`) points turns and tilts.
    pub fn drag(&mut self, dx: f64, dy: f64) {
        self.yaw += dx * 0.01;
        self.pitch = (self.pitch + dy * 0.01).clamp(0.05, 1.5);
    }
}

/// Grid cells per side.
const N: usize = 48;
/// Half the box's height, in the box's units (its width is 2).
const HEIGHT: f64 = 0.75;

type P3 = [f64; 3];

struct Camera {
    yaw: (f64, f64),
    pitch: (f64, f64),
    cx: f64,
    cy: f64,
    size: f64,
}

impl Camera {
    /// Screen point and depth (bigger is farther).
    fn project(&self, p: P3) -> (f32, f32, f64) {
        let (sy, cy) = self.yaw;
        let (sp, cp) = self.pitch;
        let x = p[0] * cy - p[1] * sy;
        let y = p[0] * sy + p[1] * cy;
        let up = y * sp + p[2] * cp;
        let depth = y * cp - p[2] * sp;
        let f = 5.0 / (5.0 + depth);
        (
            (self.cx + x * f * self.size) as f32,
            (self.cy - up * f * self.size) as f32,
            depth,
        )
    }
}

/// Height → colour, from deep indigo through magenta to warm yellow.
pub fn ramp(t: f64) -> [f64; 3] {
    const STOPS: [[f64; 3]; 5] = [
        [40.0, 30.0, 120.0],
        [110.0, 50.0, 190.0],
        [210.0, 70.0, 150.0],
        [250.0, 140.0, 80.0],
        [250.0, 225.0, 110.0],
    ];
    let t = t.clamp(0.0, 1.0) * (STOPS.len() - 1) as f64;
    let i = (t.floor() as usize).min(STOPS.len() - 2);
    let f = t - i as f64;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    [
        a[0] + (b[0] - a[0]) * f,
        a[1] + (b[1] - a[1]) * f,
        a[2] + (b[2] - a[2]) * f,
    ]
}

fn sub(a: P3, b: P3) -> P3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: P3, b: P3) -> P3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: P3) -> P3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / l, a[1] / l, a[2] / l]
}

fn paint(r: f64, g: f64, b: f64, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
        a,
    ));
    p.anti_alias = true;
    p
}

/// Draws `expr` on `pm` (`scale` pixels per point). Returns labels for the axes.
pub fn draw(pm: &mut Pixmap, scale: f32, view: &View, expr: &Expr) -> Vec<Label> {
    let (w, h) = (
        pm.width() as f64 / scale as f64,
        pm.height() as f64 / scale as f64,
    );
    let t = Transform::from_scale(scale, scale);
    let cam = Camera {
        yaw: view.yaw.sin_cos(),
        pitch: view.pitch.sin_cos(),
        cx: w / 2.0,
        cy: h / 2.0 + h * 0.04,
        size: (w * 0.3).min(h * 0.36) * view.zoom,
    };

    // Sample the surface, and find its height range.
    let r = view.range;
    let mut z = vec![f64::NAN; (N + 1) * (N + 1)];
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for j in 0..=N {
        for i in 0..=N {
            let x = -r + 2.0 * r * i as f64 / N as f64;
            let y = -r + 2.0 * r * j as f64 / N as f64;
            let v = expr.eval(&Env {
                x,
                y,
                ..Default::default()
            });
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
            z[j * (N + 1) + i] = v;
        }
    }
    if !lo.is_finite() {
        lo = -1.0;
        hi = 1.0;
    }
    if hi - lo < 1e-9 {
        lo -= 1.0;
        hi += 1.0;
    }
    let world = |i: usize, j: usize| -> Option<P3> {
        let v = z[j * (N + 1) + i];
        v.is_finite().then(|| {
            [
                -1.0 + 2.0 * i as f64 / N as f64,
                -1.0 + 2.0 * j as f64 / N as f64,
                -HEIGHT + 2.0 * HEIGHT * (v - lo) / (hi - lo),
            ]
        })
    };

    // The floor grid and the box, behind everything.
    let mut floor = PathBuilder::new();
    for k in 0..=8 {
        let s = -1.0 + k as f64 * 0.25;
        for (a, b) in [
            ([s, -1.0, -HEIGHT], [s, 1.0, -HEIGHT]),
            ([-1.0, s, -HEIGHT], [1.0, s, -HEIGHT]),
        ] {
            let (ax, ay, _) = cam.project(a);
            let (bx, by, _) = cam.project(b);
            floor.move_to(ax, ay);
            floor.line_to(bx, by);
        }
    }
    if let Some(p) = floor.finish() {
        pm.stroke_path(
            &p,
            &paint(255.0, 255.0, 255.0, 22),
            &Stroke {
                width: 1.0,
                ..Default::default()
            },
            t,
            None,
        );
    }
    let mut posts = PathBuilder::new();
    for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let (ax, ay, _) = cam.project([x, y, -HEIGHT]);
        let (bx, by, _) = cam.project([x, y, HEIGHT]);
        posts.move_to(ax, ay);
        posts.line_to(bx, by);
    }
    if let Some(p) = posts.finish() {
        pm.stroke_path(
            &p,
            &paint(255.0, 255.0, 255.0, 14),
            &Stroke {
                width: 1.0,
                ..Default::default()
            },
            t,
            None,
        );
    }

    // The quads, far to near.
    let light = normalize([-0.45, -0.35, 0.82]);
    let mut quads = Vec::with_capacity(N * N);
    for j in 0..N {
        for i in 0..N {
            let (Some(a), Some(b), Some(c), Some(d)) = (
                world(i, j),
                world(i + 1, j),
                world(i + 1, j + 1),
                world(i, j + 1),
            ) else {
                continue;
            };
            let pa = cam.project(a);
            let pb = cam.project(b);
            let pc = cam.project(c);
            let pd = cam.project(d);
            let depth = (pa.2 + pb.2 + pc.2 + pd.2) / 4.0;
            let n = normalize(cross(sub(c, a), sub(d, b)));
            let lit = 0.35 + 0.65 * (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).abs();
            let height = ((a[2] + b[2] + c[2] + d[2]) / 4.0 + HEIGHT) / (2.0 * HEIGHT);
            quads.push((depth, [pa, pb, pc, pd], height, lit));
        }
    }
    quads.sort_by(|a, b| b.0.total_cmp(&a.0));
    let edge = Stroke {
        width: 0.6,
        line_cap: LineCap::Round,
        ..Default::default()
    };
    for (_, pts, height, lit) in quads {
        let mut pb = PathBuilder::new();
        pb.move_to(pts[0].0, pts[0].1);
        for p in &pts[1..] {
            pb.line_to(p.0, p.1);
        }
        pb.close();
        let Some(path) = pb.finish() else { continue };
        let c = ramp(height);
        pm.fill_path(
            &path,
            &paint(c[0] * lit, c[1] * lit, c[2] * lit, 255),
            FillRule::Winding,
            t,
            None,
        );
        pm.stroke_path(
            &path,
            &paint(c[0] * lit * 0.55, c[1] * lit * 0.55, c[2] * lit * 0.55, 160),
            &edge,
            t,
            None,
        );
    }

    // Axis names at the box's edges, and the ranges.
    let mut labels = Vec::new();
    let mut at = |p: P3, text: String| {
        let (x, y, _) = cam.project(p);
        labels.push(Label {
            x: x - 4.0,
            y: y - 8.0,
            text,
        });
    };
    // x and y by the floor's nearer edges, z on the leftmost post: never behind the
    // surface.
    let nearer = |a: P3, b: P3| {
        if cam.project(a).2 < cam.project(b).2 {
            a
        } else {
            b
        }
    };
    at(
        nearer([0.0, -1.2, -HEIGHT], [0.0, 1.2, -HEIGHT]),
        "x".into(),
    );
    at(
        nearer([-1.2, 0.0, -HEIGHT], [1.2, 0.0, -HEIGHT]),
        "y".into(),
    );
    let post = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .into_iter()
        .map(|(x, y)| [x, y, HEIGHT + 0.12])
        .min_by(|a, b| cam.project(*a).0.total_cmp(&cam.project(*b).0))
        .unwrap_or([-1.0, -1.0, HEIGHT]);
    at(post, "z".into());
    labels.push(Label {
        x: 14.0,
        y: (h - 26.0) as f32,
        text: format!(
            "x, y: −{0} … {0}    z: {1} … {2}",
            crate::expr::format(r),
            crate::expr::format((lo * 1000.0).round() / 1000.0),
            crate::expr::format((hi * 1000.0).round() / 1000.0)
        ),
    });
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_a_surface() {
        let mut pm = Pixmap::new(300, 300).unwrap();
        let f = crate::expr::parse("sin(x) * cos(y)").unwrap();
        let labels = draw(&mut pm, 1.0, &View::default(), &f);
        assert!(labels.iter().any(|l| l.text == "z"));
        assert!(
            labels.iter().any(|l| l.text.contains("z: −0.99")),
            "{labels:?}"
        );
        // Everything in view.
        assert!(
            labels.iter().all(|l| l.x >= 0.0 && l.x < 300.0),
            "{labels:?}"
        );
        // Something opaque in the middle: the surface.
        assert_eq!(pm.pixel(150, 150).unwrap().alpha(), 255);
    }

    #[test]
    fn tilt_is_limited() {
        let mut v = View::default();
        v.drag(0.0, 1000.0);
        assert_eq!(v.pitch, 1.5);
        v.drag(100.0, -1000.0);
        assert_eq!(v.pitch, 0.05);
        assert!((v.yaw - (-0.75 + 1.0)).abs() < 1e-12);
    }

    #[test]
    fn colours_run_cool_to_warm() {
        assert!(ramp(0.0)[2] > ramp(0.0)[0]);
        assert!(ramp(1.0)[0] > ramp(1.0)[2]);
    }
}
