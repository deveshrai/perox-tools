//! The cup and what's in it, as vector paths (SVG path commands in a 200 × 200 box) for
//! Slint's `Path`: computed from the drink, its size and how far brewing has got.

use std::fmt::Write;

/// The cup's inside: its walls run from (`TOP_L`..`TOP_R`) at `TOP` to (`BOT_L`..`BOT_R`)
/// at `BOTTOM`, with rounded corners at the bottom.
const TOP: f64 = 66.0;
const BOTTOM: f64 = 164.0;
const TOP_L: f64 = 46.0;
const TOP_R: f64 = 154.0;
const BOT_L: f64 = 62.0;
const BOT_R: f64 = 138.0;
const CORNER: f64 = 16.0;

/// The liquids a drink is made of.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Liquid {
    Espresso,
    Crema,
    Water,
    Milk,
    Foam,
    Chocolate,
    Cream,
}

impl Liquid {
    /// Its colour, `strength` 1–5 darkening coffee, `milk` tinting milk and foam.
    pub fn color(self, strength: u8, milk: Milk) -> [u8; 3] {
        let dark = (strength.clamp(1, 5) as f64 - 3.0) * 0.08;
        let shade = |c: [u8; 3]| c.map(|v| (v as f64 * (1.0 - dark)).clamp(0.0, 255.0) as u8);
        let tint = |c: [u8; 3]| {
            let t = milk.tint();
            [
                ((c[0] as u16 * t[0] as u16) / 255) as u8,
                ((c[1] as u16 * t[1] as u16) / 255) as u8,
                ((c[2] as u16 * t[2] as u16) / 255) as u8,
            ]
        };
        match self {
            Liquid::Espresso => shade([0x3b, 0x20, 0x14]),
            Liquid::Crema => shade([0xc4, 0x84, 0x48]),
            Liquid::Water => shade([0x5a, 0x33, 0x1e]),
            Liquid::Milk => tint([0xe9, 0xd6, 0xc0]),
            Liquid::Foam => tint([0xfb, 0xf3, 0xe8]),
            Liquid::Chocolate => [0x5a, 0x2e, 0x1a],
            Liquid::Cream => [0xff, 0xfa, 0xf2],
        }
    }

    pub fn is_milk(self) -> bool {
        matches!(self, Liquid::Milk | Liquid::Foam)
    }

    /// Foam and crema sit in soft waves; liquids lie flat.
    pub fn wavy(self) -> bool {
        matches!(self, Liquid::Foam | Liquid::Cream | Liquid::Crema)
    }
}

/// The milk chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Milk {
    Whole,
    Oat,
    Almond,
    Skimmed,
    None,
}

impl Milk {
    pub const ALL: [Milk; 5] = [
        Milk::Whole,
        Milk::Oat,
        Milk::Almond,
        Milk::Skimmed,
        Milk::None,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Milk::Whole => "whole milk",
            Milk::Oat => "oat milk",
            Milk::Almond => "almond milk",
            Milk::Skimmed => "skimmed milk",
            Milk::None => "no milk",
        }
    }

    fn tint(self) -> [u8; 3] {
        match self {
            Milk::Whole | Milk::None => [255, 255, 255],
            Milk::Oat => [250, 236, 214],
            Milk::Almond => [246, 232, 218],
            Milk::Skimmed => [246, 248, 252],
        }
    }
}

/// The y of a fill level (0 = empty, 1 = to the brim).
pub fn level_y(f: f64) -> f64 {
    BOTTOM - f.clamp(0.0, 1.0) * (BOTTOM - TOP - 6.0)
}

/// Where the walls are at height `y`.
fn walls(y: f64) -> (f64, f64) {
    let t = (y - TOP) / (BOTTOM - TOP);
    (TOP_L + (BOT_L - TOP_L) * t, TOP_R + (BOT_R - TOP_R) * t)
}

fn n(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_owned).unwrap_or(s)
}

/// The region between fill levels `from` and `to`: up the walls, across the top (in
/// waves for foam), and round the bottom corners if it starts at the bottom.
pub fn layer(from: f64, to: f64, wavy: bool, phase: f64) -> String {
    let (y0, y1) = (level_y(from), level_y(to));
    if y0 - y1 < 0.3 {
        return String::new();
    }
    let (l0, r0) = walls(y0);
    let (l1, r1) = walls(y1);
    let mut s = String::new();
    // The bottom corners' height: less when the layer is thinner than them.
    let c = CORNER.min(y0 - y1);
    // Left wall, bottom up.
    if from <= 0.0 {
        let _ = write!(s, "M {} {} ", n(l0 + CORNER), n(y0));
        let _ = write!(
            s,
            "Q {} {} {} {} ",
            n(l0),
            n(y0),
            n(l0 + 1.5 * c / CORNER),
            n(y0 - c)
        );
    } else {
        let _ = write!(s, "M {} {} ", n(l0), n(y0));
    }
    let _ = write!(s, "L {} {} ", n(l1), n(y1));
    // The top: a few soft bumps for foam, flat for liquids.
    if wavy {
        let bumps = 6;
        let w = (r1 - l1) / bumps as f64;
        for i in 0..bumps {
            let x0 = l1 + w * i as f64;
            let lift = 2.2 + 1.2 * ((i as f64 * 1.7 + phase).sin());
            let _ = write!(
                s,
                "Q {} {} {} {} ",
                n(x0 + w / 2.0),
                n(y1 - lift * 2.0),
                n(x0 + w),
                n(y1)
            );
        }
    } else {
        let _ = write!(s, "L {} {} ", n(r1), n(y1));
    }
    // Right wall, down.
    if from <= 0.0 {
        let _ = write!(s, "L {} {} ", n(r0 - 1.5 * c / CORNER), n(y0 - c));
        let _ = write!(s, "Q {} {} {} {} ", n(r0), n(y0), n(r0 - CORNER), n(y0));
    } else {
        let _ = write!(s, "L {} {} ", n(r0), n(y0));
    }
    s.push('Z');
    s
}

/// The cup: its body (a rim, tapered walls, a rounded bottom) and the handle.
pub fn cup_outline() -> String {
    let mut s = String::new();
    let _ = write!(
        s,
        "M 40 60 L 160 60 L 143 170 Q 141 178 132 178 L 68 178 Q 59 178 57 170 Z "
    );
    // The handle: a loop on the right.
    let _ = write!(
        s,
        "M 156 82 C 190 80 192 132 148 140 L 150 128 C 176 122 176 94 154 95 Z"
    );
    s
}

/// The saucer under it.
pub fn saucer() -> String {
    "M 14 180 Q 100 200 186 180 Q 186 190 170 192 L 30 192 Q 14 190 14 180 Z".into()
}

/// Steam: `count` wisps rising from the cup, swaying with `t` (seconds).
pub fn steam(t: f64, count: usize) -> Vec<String> {
    (0..count)
        .map(|i| {
            let x = 78.0 + i as f64 * 22.0;
            let p = t * 1.6 + i as f64 * 1.9;
            let sway = |k: f64| 7.0 * (p + k).sin();
            format!(
                "M {} 52 C {} 38 {} 30 {} 18 S {} -2 {} -10",
                n(x),
                n(x + sway(0.0)),
                n(x - sway(1.0)),
                n(x + sway(2.0) * 0.6),
                n(x - sway(3.0)),
                n(x + sway(4.0) * 0.5)
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_follow_the_walls() {
        let bottom = layer(0.0, 0.3, false, 0.0);
        assert!(bottom.starts_with("M 78 164 Q 62 164"), "{bottom}");
        assert!(bottom.ends_with('Z'));
        let top = layer(0.3, 0.6, true, 0.0);
        assert_eq!(top.matches('Q').count(), 6, "{top}");
        assert_eq!(layer(0.5, 0.5, false, 0.0), "");
        // Fuller is higher (smaller y).
        assert!(level_y(0.8) < level_y(0.2));
    }

    #[test]
    fn strength_darkens_and_milk_tints() {
        let mild = Liquid::Espresso.color(1, Milk::Whole);
        let strong = Liquid::Espresso.color(5, Milk::Whole);
        assert!(strong[0] < mild[0]);
        assert_ne!(
            Liquid::Milk.color(3, Milk::Oat),
            Liquid::Milk.color(3, Milk::Whole)
        );
        assert_eq!(steam(0.0, 3).len(), 3);
    }
}
