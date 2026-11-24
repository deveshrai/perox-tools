//! PeroxIDE's icon and opening animation, drawn in code so every size and every frame
//! comes from one drawing.
//!
//! The picture, side on: an anvil, rusted at the heel and polished chrome at the horn, and
//! a chrome bottle pouring liquid chrome onto its middle. Two shades only: chrome and
//! rust. The animation starts with the anvil all rust; the bottle tips, the stream lands,
//! and chrome spreads from where it lands across the horn side. Its last frame is the icon.
//!
//! Coordinates are a 1024 × 1024 canvas; [`Art`] scales them to any size.

use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, Point,
    PremultipliedColorU8, RadialGradient, Rect, Shader, SpreadMode, Stroke, Transform,
};

mod noise;
use noise::fbm;

pub use tiny_skia;

/// Where the stream lands, on the anvil's face.
const POUR: (f32, f32) = (520.0, 452.0);
/// The face's top edge.
const FACE: f32 = 452.0;

/// How much to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// The whole scene: anvil, bottle, stream, glints.
    Full,
    /// Just the anvil, larger and bolder: for icons of 64 px and less.
    Small,
}

/// The scene at one size, ready to draw any frame of the animation.
#[allow(missing_debug_implementations)]
pub struct Art {
    size: u32,
    detail: Detail,
    /// Canvas → pixels.
    xf: Transform,
    /// The anvil in chrome, and in rust (same coverage).
    chrome: Pixmap,
    rust: Pixmap,
    /// For each pixel, when (0–1 of the spread) chrome reaches it; `f32::INFINITY` where
    /// it stays rust.
    when: Vec<f32>,
}

/// Animation timing, as fractions of the whole (see [`Art::frame`]).
pub mod timing {
    /// The whole animation, in seconds.
    pub const SECONDS: f32 = 2.4;
    pub(crate) const TILT: (f32, f32) = (0.0, 0.22);
    pub(crate) const STREAM: (f32, f32) = (0.18, 0.34);
    pub(crate) const SPREAD: (f32, f32) = (0.32, 0.82);
    pub(crate) const GLINT: (f32, f32) = (0.78, 0.95);
}

fn ramp(t: f32, (a, b): (f32, f32)) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

fn ease(x: f32) -> f32 {
    // Ease out cubic.
    1.0 - (1.0 - x).powi(3)
}

fn rgb(hex: u32) -> Color {
    Color::from_rgba8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255)
}

fn rgba(hex: u32, a: f32) -> Color {
    let mut c = rgb(hex);
    c.set_alpha(a.clamp(0.0, 1.0));
    c
}

fn stops(list: &[(f32, u32)]) -> Vec<GradientStop> {
    list.iter()
        .map(|&(p, c)| GradientStop::new(p, rgb(c)))
        .collect()
}

fn linear(a: (f32, f32), b: (f32, f32), list: &[(f32, u32)]) -> Shader<'static> {
    LinearGradient::new(
        Point::from_xy(a.0, a.1),
        Point::from_xy(b.0, b.1),
        stops(list),
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(rgb(list[0].1)))
}

fn paint(shader: Shader<'static>) -> Paint<'static> {
    Paint {
        shader,
        anti_alias: true,
        ..Default::default()
    }
}

fn solid(c: Color) -> Paint<'static> {
    paint(Shader::SolidColor(c))
}

/// The anvil's outline: heel on the left, horn on the right.
pub fn anvil() -> Path {
    let mut p = PathBuilder::new();
    p.move_to(112.0, FACE); // heel, top
    p.line_to(700.0, FACE); // the face
    p.line_to(700.0, 474.0); // step down to the horn
    p.line_to(746.0, 474.0);
    p.quad_to(850.0, 470.0, 950.0, 486.0); // horn, top, to the tip
    p.cubic_to(900.0, 506.0, 800.0, 590.0, 708.0, 600.0); // horn, underside
    p.cubic_to(664.0, 612.0, 640.0, 664.0, 636.0, 712.0); // body into the waist
    p.cubic_to(642.0, 770.0, 690.0, 800.0, 738.0, 810.0); // flare to the foot
    p.line_to(738.0, 882.0);
    p.line_to(606.0, 882.0);
    p.quad_to(512.0, 846.0, 418.0, 882.0); // the arch between the feet
    p.line_to(286.0, 882.0);
    p.line_to(286.0, 810.0);
    p.cubic_to(334.0, 800.0, 382.0, 770.0, 388.0, 712.0); // flare, left
    p.cubic_to(392.0, 664.0, 410.0, 622.0, 428.0, 600.0); // waist up to the heel
    p.cubic_to(380.0, 566.0, 260.0, 538.0, 168.0, 530.0); // heel, underside
    p.line_to(140.0, 530.0);
    p.line_to(112.0, 500.0);
    p.close();
    p.finish().expect("anvil path")
}

/// Polished chrome, top to bottom: bright face, dark under-edges, a band of reflection.
fn chrome_shader() -> Shader<'static> {
    linear(
        (0.0, FACE),
        (0.0, 882.0),
        &[
            (0.000, 0xfdfeff),
            (0.030, 0xe2e6ea),
            (0.048, 0x8e959d),
            (0.070, 0xf1f3f5),
            (0.190, 0xc9ced3),
            (0.205, 0x5d646c),
            (0.225, 0xdfe3e7),
            (0.420, 0x9aa1a8),
            (0.600, 0xf2f4f6),
            (0.780, 0x7a828a),
            (0.800, 0xe8ebee),
            (1.000, 0x8b929a),
        ],
    )
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Rust at a canvas point: blotchy oxide, pits, a few bright flakes.
fn rust_color(x: f32, y: f32) -> [f32; 3] {
    let dark = [74.0, 28.0, 11.0];
    let mid = [156.0, 63.0, 20.0];
    let orange = [204.0, 98.0, 33.0];
    let light = [230.0, 140.0, 62.0];
    let n = fbm(x / 70.0, y / 70.0, 5) * 0.82 + fbm(x / 14.0, y / 14.0, 2) * 0.18;
    let n = ((n - 0.5) * 1.6 + 0.5).clamp(0.0, 1.0);
    let c = if n < 0.38 {
        mix(dark, mid, (n / 0.38).powf(0.7))
    } else if n < 0.62 {
        mix(mid, orange, (n - 0.38) / 0.24)
    } else {
        mix(orange, light, ((n - 0.62) / 0.38).min(1.0))
    };
    // Pits.
    let pit = fbm(x / 6.0 + 40.0, y / 6.0, 1);
    if pit > 0.82 { mix(c, dark, 0.6) } else { c }
}

/// Where chrome ends up (right of a ragged line through the pour point), and when the
/// spread reaches a point.
fn chrome_time(x: f32, y: f32) -> f32 {
    let edge = 488.0 + 46.0 * (fbm(3.1, y / 34.0, 3) - 0.5) + (y - FACE) * 0.04;
    let ragged = 64.0 * (fbm(x / 16.0, y / 16.0 + 9.0, 3) - 0.5)
        + 26.0 * (noise::value(x / 3.5, y / 3.5) - 0.5);
    if x + ragged < edge {
        return f32::INFINITY;
    }
    // Flecks of rust left in the chrome near the edge.
    let near = (1.0 - (x - edge) / 70.0).clamp(0.0, 1.0);
    if near > 0.0 && noise::value(x / 5.0 + 70.0, y / 5.0) > 1.0 - 0.35 * near {
        return f32::INFINITY;
    }
    let d = ((x - POUR.0).powi(2) + (y - POUR.1).powi(2)).sqrt();
    (d / 470.0 + 0.10 * (fbm(x / 30.0, y / 30.0 + 50.0, 2) - 0.5)).max(0.0)
}

impl Art {
    /// The scene at `size` × `size` pixels, the whole canvas.
    pub fn new(size: u32, detail: Detail) -> Self {
        let s = size as f32 / 1024.0;
        Self::framed(size, detail, Transform::from_scale(s, s))
    }

    /// The scene drawn through `xf` (canvas → pixels).
    pub fn framed(size: u32, detail: Detail, xf: Transform) -> Self {
        let anvil = anvil();
        let mut chrome = Pixmap::new(size, size).expect("size");
        chrome.fill_path(&anvil, &paint(chrome_shader()), FillRule::Winding, xf, None);
        // Diagonal reflections across the chrome.
        let mut clip = tiny_skia::Mask::new(size, size).expect("size");
        clip.fill_path(&anvil, FillRule::Winding, true, xf);
        for &(x, w, a) in &[
            (560.0, 70.0, 0.22),
            (660.0, 22.0, 0.30),
            (800.0, 40.0, 0.16),
        ] {
            let mut b = PathBuilder::new();
            b.move_to(x, 440.0);
            b.line_to(x + w, 440.0);
            b.line_to(x + w - 260.0, 900.0);
            b.line_to(x - 260.0, 900.0);
            b.close();
            chrome.fill_path(
                &b.finish().expect("band"),
                &solid(rgba(0xffffff, a)),
                FillRule::Winding,
                xf,
                Some(&clip),
            );
        }
        // The rust layer: the anvil's coverage, rust coloured but keeping the chrome's
        // light and shade so the faces still read.
        let mut rust = chrome.clone();
        let inv = xf.invert().unwrap_or_default();
        let mut when = vec![f32::INFINITY; (size * size) as usize];
        for (i, px) in rust.pixels_mut().iter_mut().enumerate() {
            let a = px.alpha();
            if a == 0 {
                continue;
            }
            let mut p = Point::from_xy(
                (i as u32 % size) as f32 + 0.5,
                (i as u32 / size) as f32 + 0.5,
            );
            inv.map_points(std::slice::from_mut(&mut p));
            let lum = px.red() as f32 / a as f32;
            let shade = 0.45 + 0.75 * lum;
            let c = rust_color(p.x, p.y);
            let af = a as f32 / 255.0;
            let ch = |v: f32| ((v * shade).min(255.0) * af) as u8;
            *px = PremultipliedColorU8::from_rgba(ch(c[0]), ch(c[1]), ch(c[2]), a).unwrap_or(*px);
            when[i] = chrome_time(p.x, p.y);
        }
        Self {
            size,
            detail,
            xf,
            chrome,
            rust,
            when,
        }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    /// Frame `t` (0 = start, 1 = the finished picture) on a transparent background.
    pub fn frame(&self, t: f32) -> Pixmap {
        let t = t.clamp(0.0, 1.0);
        let mut out = Pixmap::new(self.size, self.size).expect("size");
        let full = self.detail == Detail::Full;
        let spread = if full {
            ease(ramp(t, timing::SPREAD))
        } else {
            1.0
        };
        let xf = self.xf;

        // The anvil: rust, giving way to chrome.
        {
            let dst = out.pixels_mut();
            let c = self.chrome.pixels();
            let r = self.rust.pixels();
            for i in 0..dst.len() {
                dst[i] = if self.when[i] <= spread { c[i] } else { r[i] };
            }
        }
        let anvil = anvil();
        // The face's top edge catches the light; the groove under the face block.
        let mut sheen = PathBuilder::new();
        sheen.push_rect(Rect::from_xywh(112.0, FACE, 588.0, 5.0).expect("rect"));
        out.fill_path(
            &sheen.finish().expect("sheen"),
            &solid(rgba(0xffffff, 0.35)),
            FillRule::Winding,
            xf,
            None,
        );
        let mut groove = PathBuilder::new();
        groove.move_to(150.0, 540.0);
        groove.line_to(708.0, 540.0);
        out.stroke_path(
            &groove.finish().expect("groove"),
            &solid(rgba(0x000000, 0.28)),
            &Stroke {
                width: 3.0,
                ..Default::default()
            },
            xf,
            None,
        );
        out.stroke_path(
            &anvil,
            &solid(rgb(0x1b1d20)),
            &Stroke {
                width: if full { 7.0 } else { 16.0 },
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            },
            xf,
            None,
        );

        if full {
            self.pour(&mut out, t);
        }
        // Glints: on the face corner, the horn tip, a foot, the pool.
        let g = if full { ramp(t, timing::GLINT) } else { 1.0 };
        if g > 0.0 {
            let glints: &[(f32, f32, f32)] = if full {
                &[
                    (700.0, 456.0, 1.0),
                    (938.0, 488.0, 0.8),
                    (734.0, 812.0, 0.7),
                    (566.0, 446.0, 0.75),
                    (628.0, 262.0, 0.6),
                ]
            } else {
                &[(700.0, 456.0, 1.6), (936.0, 488.0, 1.2)]
            };
            for (k, &(x, y, s)) in glints.iter().enumerate() {
                // Each one flares in turn.
                let local = ((g * 1.6) - k as f32 * 0.15).clamp(0.0, 1.0);
                if local > 0.0 {
                    glint(&mut out, xf, x, y, 34.0 * s * ease(local));
                }
            }
        }
        out
    }

    /// The bottle, the stream, the pool and drips, and the splash.
    fn pour(&self, out: &mut Pixmap, t: f32) {
        let xf = self.xf;
        // The bottle tips in from the top right.
        let tilt = ease(ramp(t, timing::TILT));
        let angle = -8.0 + (-35.0 + 8.0) * tilt; // degrees: the body up and to the right
        let mouth = (600.0 + 260.0 * (1.0 - tilt), 268.0 - 300.0 * (1.0 - tilt));
        bottle(out, xf, mouth, angle);

        // The stream: out of the mouth, down-left, then falling onto the face.
        let s = ease(ramp(t, timing::STREAM));
        if s > 0.0 {
            let (sin, cos) = angle.to_radians().sin_cos();
            let at = |u: f32, v: f32| (mouth.0 + u * cos - v * sin, mouth.1 + u * sin + v * cos);
            let p0 = at(-4.0, 10.0);
            let p1 = at(-70.0, 30.0);
            let p2 = (POUR.0, POUR.1 - 130.0);
            let p3 = (POUR.0, FACE + 2.0);
            let bez = |k: f32| {
                let m = 1.0 - k;
                let a = m * m * m;
                let b = 3.0 * m * m * k;
                let c = 3.0 * m * k * k;
                let d = k * k * k;
                (
                    a * p0.0 + b * p1.0 + c * p2.0 + d * p3.0,
                    a * p0.1 + b * p1.1 + c * p2.1 + d * p3.1,
                )
            };
            let n = 48;
            let pts: Vec<(f32, f32)> = (0..=n).map(|i| bez(s * i as f32 / n as f32)).collect();
            // A ribbon around the centre line: `w(k)` half-width, shifted `off(k)` sideways.
            let ribbon = |w: &dyn Fn(f32) -> f32, off: &dyn Fn(f32) -> f32| {
                let side = |i: usize| {
                    let a = pts[i.saturating_sub(1)];
                    let b = pts[(i + 1).min(n)];
                    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                    let l = (dx * dx + dy * dy).sqrt().max(1e-3);
                    (-dy / l, dx / l)
                };
                let mut c = PathBuilder::new();
                for (i, pt) in pts.iter().enumerate() {
                    let k = s * i as f32 / n as f32;
                    let (nx, ny) = side(i);
                    let d = off(k) - w(k);
                    let (x, y) = (pt.0 + nx * d, pt.1 + ny * d);
                    if i == 0 {
                        c.move_to(x, y);
                    } else {
                        c.line_to(x, y);
                    }
                }
                for i in (0..=n).rev() {
                    let k = s * i as f32 / n as f32;
                    let (nx, ny) = side(i);
                    let d = off(k) + w(k);
                    c.line_to(pts[i].0 + nx * d, pts[i].1 + ny * d);
                }
                c.close();
                c.finish()
            };
            // Thick at the mouth, thinning as it falls, spreading where it lands.
            let half = |k: f32| 15.0 - 7.0 * k + 10.0 * ((k - 0.9) / 0.1).clamp(0.0, 1.0).powi(2);
            let layers: [(f32, f32, f32, Color); 4] = [
                (3.0, 0.0, 0.0, rgb(0x1b1d20)),
                (0.0, 0.0, 0.0, rgb(0x8f979f)),
                (-5.0, 0.62, 0.0, rgb(0xdde1e5)),
                (-8.0, 0.25, 0.35, rgb(0xffffff)),
            ];
            for (grow, scale, shift, c) in layers {
                let w = |k: f32| {
                    if scale == 0.0 {
                        half(k) + grow
                    } else {
                        half(k) * scale
                    }
                };
                let off = |k: f32| {
                    if scale == 0.0 {
                        0.0
                    } else {
                        -half(k) * shift + grow * 0.3
                    }
                };
                if let Some(path) = ribbon(&w, &off) {
                    out.fill_path(&path, &solid(c), FillRule::Winding, xf, None);
                }
            }
        }

        // The pool on the face, and drips down the front.
        let pool = ease(ramp(t, (timing::STREAM.1 - 0.02, timing::SPREAD.0 + 0.25)));
        if pool > 0.0 {
            let half = 18.0 + 66.0 * pool;
            let rise = 14.0 + 12.0 * pool;
            let (x0, x1) = (POUR.0 - half - 10.0, POUR.0 + half + 10.0);
            let lip = FACE + 4.0 + 10.0 * pool; // how far it hangs over the front edge
            let mut drips = Vec::new();
            for &(dx, len, w, start) in &[
                (44.0, 62.0, 11.0, 0.15),
                (-6.0, 112.0, 14.0, 0.0),
                (-52.0, 44.0, 10.0, 0.25),
            ] {
                let x = POUR.0 + dx;
                let k = ease(((pool - start) / (1.0 - start)).clamp(0.0, 1.0));
                if k > 0.0 && x - w * 1.8 > x0 + 8.0 && x + w * 1.8 < x1 - 8.0 {
                    drips.push((x, (len * k).max(lip - FACE + 2.0), w));
                }
            }
            let mut p = PathBuilder::new();
            p.move_to(x0, FACE + 2.0);
            p.cubic_to(
                POUR.0 - half,
                FACE - rise,
                POUR.0 + half,
                FACE - rise,
                x1,
                FACE + 2.0,
            );
            p.quad_to(x1, lip, x1 - 10.0, lip);
            // Along the front edge, right to left, with the drips hanging from it.
            for &(x, len, w) in &drips {
                p.line_to(x + w * 1.8, lip);
                p.quad_to(x + w, lip, x + w * 0.85, FACE + len);
                p.cubic_to(
                    x + w * 1.3,
                    FACE + len + w * 1.9,
                    x - w * 1.3,
                    FACE + len + w * 1.9,
                    x - w * 0.85,
                    FACE + len,
                );
                p.quad_to(x - w, lip, x - w * 1.8, lip);
            }
            p.line_to(x0 + 10.0, lip);
            p.quad_to(x0, lip, x0, FACE + 2.0);
            p.close();
            let path = p.finish().expect("pool");
            // Outline first, so the fill covers where the drips meet the pool.
            out.stroke_path(
                &path,
                &solid(rgb(0x1b1d20)),
                &Stroke {
                    width: 6.0,
                    line_join: tiny_skia::LineJoin::Round,
                    ..Default::default()
                },
                xf,
                None,
            );
            let sh = linear(
                (POUR.0 - 90.0, FACE - 20.0),
                (POUR.0 + 60.0, FACE + 120.0),
                &[
                    (0.0, 0x9aa1a8),
                    (0.25, 0xffffff),
                    (0.4, 0xd3d8dc),
                    (0.6, 0x7d858d),
                    (0.8, 0xe6e9ec),
                    (1.0, 0x8a929a),
                ],
            );
            out.fill_path(&path, &paint(sh), FillRule::Winding, xf, None);
        }

        // Sparks where it lands: rust-orange flecks flying out and falling.
        let sp = ramp(t, (timing::STREAM.1 - 0.04, timing::SPREAD.0 + 0.3));
        if sp > 0.0 && sp < 1.0 {
            for i in 0..22 {
                let a = noise::hash(i, 7) * std::f32::consts::PI; // upward half
                let v = 120.0 + 260.0 * noise::hash(i, 13);
                let start = noise::hash(i, 21) * 0.4;
                let k = ((sp - start) / 0.6).clamp(0.0, 1.0);
                if k <= 0.0 || k >= 1.0 {
                    continue;
                }
                let x = POUR.0 + a.cos() * v * k;
                let y = FACE - 6.0 - a.sin() * v * k + 260.0 * k * k;
                let fade = 1.0 - k;
                let r = 2.5 + 2.5 * noise::hash(i, 3);
                let mut c = PathBuilder::new();
                c.push_circle(x, y, r);
                if let Some(path) = c.finish() {
                    out.fill_path(
                        &path,
                        &solid(rgba(if i % 3 == 0 { 0xffd2a0 } else { 0xe0782a }, fade)),
                        FillRule::Winding,
                        xf,
                        None,
                    );
                }
            }
        }
    }

    /// The finished picture on the icon's rounded square, the way an app icon is shaped.
    pub fn icon(&self) -> Pixmap {
        let size = self.size as f32;
        let mut out = Pixmap::new(self.size, self.size).expect("size");
        // Apple's grid: an 824 / 1024 square, corners 185.
        let (inset, radius) = (size * 100.0 / 1024.0, size * 185.0 / 1024.0);
        let tile = rounded_rect(inset, inset, size - 2.0 * inset, size - 2.0 * inset, radius);
        out.fill_path(
            &tile,
            &paint(linear(
                (0.0, inset),
                (0.0, size - inset),
                &[(0.0, 0x24262a), (1.0, 0x0f1012)],
            )),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        // The scene, kept inside the tile (the bottle runs off its edge).
        let mut mask = tiny_skia::Mask::new(self.size, self.size).expect("size");
        mask.fill_path(&tile, FillRule::Winding, true, Transform::identity());
        let scene = self.frame(1.0);
        out.draw_pixmap(
            0,
            0,
            scene.as_ref(),
            &tiny_skia::PixmapPaint::default(),
            Transform::identity(),
            Some(&mask),
        );
        out.stroke_path(
            &tile,
            &solid(rgba(0xffffff, 0.08)),
            &Stroke {
                width: (size / 512.0).max(1.0),
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
        out
    }
}

/// The icon at `px` pixels: the full scene from 128 px up, the anvil alone below.
pub fn icon(px: u32) -> Pixmap {
    let s = px as f32 / 1024.0;
    let art = if px <= 64 {
        // The anvil alone, filling the tile.
        let k = 0.95;
        Art::framed(
            px,
            Detail::Small,
            Transform::from_row(
                k * s,
                0.0,
                0.0,
                k * s,
                (512.0 - 531.0 * k) * s,
                (512.0 - 667.0 * k) * s,
            ),
        )
    } else {
        // The scene, a little smaller, the anvil centred; the bottle runs off the corner.
        let k = 0.80;
        Art::framed(
            px,
            Detail::Full,
            Transform::from_row(
                k * s,
                0.0,
                0.0,
                k * s,
                (512.0 - 531.0 * k) * s,
                (512.0 - 560.0 * k) * s,
            ),
        )
    };
    art.icon()
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
    let k = 0.552_284_8 * r;
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    p.close();
    p.finish().expect("rounded rect")
}

/// A chrome bottle, its mouth at `mouth`, its body along `angle` degrees.
fn bottle(out: &mut Pixmap, xf: Transform, mouth: (f32, f32), angle: f32) {
    // Drawn along +x from the mouth, then rotated into place.
    let place = xf.pre_translate(mouth.0, mouth.1).pre_rotate(angle);
    let mut b = PathBuilder::new();
    // Neck and shoulder, then the body, which runs off the canvas.
    b.move_to(0.0, -30.0);
    b.line_to(70.0, -30.0);
    b.cubic_to(110.0, -30.0, 120.0, -104.0, 170.0, -110.0);
    b.line_to(720.0, -110.0);
    b.line_to(720.0, 110.0);
    b.line_to(170.0, 110.0);
    b.cubic_to(120.0, 104.0, 110.0, 30.0, 70.0, 30.0);
    b.line_to(0.0, 30.0);
    b.close();
    let body = b.finish().expect("bottle");
    let across = |list: &[(f32, u32)]| {
        LinearGradient::new(
            Point::from_xy(0.0, -110.0),
            Point::from_xy(0.0, 110.0),
            stops(list),
            SpreadMode::Pad,
            Transform::identity(),
        )
        .expect("gradient")
    };
    let sh = across(&[
        (0.00, 0x6a717a),
        (0.12, 0xe9ecef),
        (0.22, 0xffffff),
        (0.34, 0xb4bac1),
        (0.50, 0x8b939b),
        (0.64, 0xdde1e5),
        (0.82, 0x6f777f),
        (1.00, 0x40464d),
    ]);
    out.fill_path(&body, &paint(sh), FillRule::Winding, place, None);
    // Facets along the body.
    for y in [-62.0, 58.0] {
        let mut l = PathBuilder::new();
        l.move_to(190.0, y);
        l.line_to(720.0, y);
        out.stroke_path(
            &l.finish().expect("facet"),
            &solid(rgba(0xffffff, 0.45)),
            &Stroke {
                width: 3.0,
                ..Default::default()
            },
            place,
            None,
        );
    }
    out.stroke_path(
        &body,
        &solid(rgb(0x1b1d20)),
        &Stroke {
            width: 6.0,
            line_join: tiny_skia::LineJoin::Round,
            ..Default::default()
        },
        place,
        None,
    );
    // The lip and the threads on the neck.
    for (x, w, h) in [
        (0.0, 16.0, 36.0),
        (24.0, 9.0, 33.0),
        (40.0, 9.0, 33.0),
        (56.0, 9.0, 33.0),
    ] {
        let r = rounded_rect(x, -h, w, 2.0 * h, 4.0);
        let sh = across(&[
            (0.32, 0x7d858d),
            (0.42, 0xffffff),
            (0.55, 0xc2c8ce),
            (0.7, 0x5d656d),
        ]);
        out.fill_path(&r, &paint(sh), FillRule::Winding, place, None);
        out.stroke_path(
            &r,
            &solid(rgb(0x1b1d20)),
            &Stroke {
                width: 3.0,
                ..Default::default()
            },
            place,
            None,
        );
    }
}

/// A four-pointed glint with a soft glow.
fn glint(out: &mut Pixmap, xf: Transform, x: f32, y: f32, r: f32) {
    if let Some(glow) = RadialGradient::new(
        Point::from_xy(x, y),
        0.0,
        Point::from_xy(x, y),
        r * 0.9,
        vec![
            GradientStop::new(0.0, rgba(0xffffff, 0.75)),
            GradientStop::new(1.0, rgba(0xffffff, 0.0)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        let mut c = PathBuilder::new();
        c.push_circle(x, y, r);
        if let Some(path) = c.finish() {
            out.fill_path(&path, &paint(glow), FillRule::Winding, xf, None);
        }
    }
    let k = r * 0.12;
    let mut s = PathBuilder::new();
    s.move_to(x, y - r);
    s.quad_to(x + k, y - k, x + r, y);
    s.quad_to(x + k, y + k, x, y + r);
    s.quad_to(x - k, y + k, x - r, y);
    s.quad_to(x - k, y - k, x, y - r);
    s.close();
    if let Some(path) = s.finish() {
        out.fill_path(&path, &solid(rgb(0xffffff)), FillRule::Winding, xf, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spread_turns_rust_to_chrome_on_the_horn_side_only() {
        let art = Art::new(128, Detail::Full);
        let at = |pm: &Pixmap, x: f32, y: f32| {
            let s = 128.0 / 1024.0;
            pm.pixel((x * s) as u32, (y * s) as u32).expect("pixel")
        };
        let start = art.frame(0.0);
        let end = art.frame(1.0);
        // The horn: rust at first (red over blue), chrome at the end (grey).
        let horn0 = at(&start, 820.0, 500.0);
        let horn1 = at(&end, 820.0, 500.0);
        assert!(horn0.red() > horn0.blue() + 40, "{horn0:?}");
        assert!(
            (horn1.red() as i32 - horn1.blue() as i32).abs() < 20,
            "{horn1:?}"
        );
        // The heel stays rust.
        let heel = at(&end, 200.0, 490.0);
        assert!(heel.red() > heel.blue() + 40, "{heel:?}");
        // Outside the anvil and the bottle: nothing.
        assert_eq!(at(&end, 40.0, 960.0).alpha(), 0);
    }

    #[test]
    fn icons_at_every_size() {
        for px in [16, 32, 64, 128, 256] {
            let i = icon(px);
            assert_eq!(i.width(), px);
            // The tile's corner is transparent, its middle isn't.
            assert_eq!(i.pixel(0, 0).expect("px").alpha(), 0);
            assert_eq!(i.pixel(px / 2, px / 2).expect("px").alpha(), 255);
        }
    }
}
