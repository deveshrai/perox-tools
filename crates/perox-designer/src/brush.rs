//! Gradients the inspector can edit: `@linear-gradient(<angle>deg, <color> <pos>%, …)`.
//! Anything else (radial gradients, colours from expressions) is edited as source.

use perox_slint::Value;

/// A linear gradient: angle in degrees (0 = bottom to top, 90 = left to right) and
/// colour stops `(rgba, position 0–1)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Linear {
    pub angle: f32,
    pub stops: Vec<([u8; 4], f32)>,
}

impl Linear {
    /// Reads `@linear-gradient(…)`; `None` if it isn't one this editor understands.
    pub fn parse(src: &str) -> Option<Self> {
        let inner = src
            .trim()
            .strip_prefix("@linear-gradient(")?
            .strip_suffix(')')?;
        let mut parts = inner.split(',').map(str::trim);
        let angle = parts.next()?.strip_suffix("deg")?.trim().parse().ok()?;
        let mut stops = Vec::new();
        for p in parts {
            let (color, pos) = match p.rsplit_once(' ') {
                Some((c, pos)) if pos.ends_with('%') => (
                    c.trim(),
                    pos.trim_end_matches('%').parse::<f32>().ok()? / 100.0,
                ),
                _ => (p, f32::NAN),
            };
            let Value::Color(rgba) = Value::parse(color) else {
                return None;
            };
            stops.push((rgba, pos));
        }
        if stops.len() < 2 {
            return None;
        }
        // Stops without a position spread evenly.
        let n = stops.len() - 1;
        for (i, s) in stops.iter_mut().enumerate() {
            if s.1.is_nan() {
                s.1 = i as f32 / n as f32;
            }
        }
        Some(Self { angle, stops })
    }

    pub fn to_source(&self) -> String {
        let stops: Vec<String> = self
            .stops
            .iter()
            .map(|(c, p)| {
                format!(
                    "{} {}%",
                    Value::Color(*c).to_source(),
                    (p * 100.0).round() as i32
                )
            })
            .collect();
        format!(
            "@linear-gradient({}deg, {})",
            self.angle.round() as i32,
            stops.join(", ")
        )
    }

    /// A gentle gradient from `rgba` to a darker shade of it, top to bottom.
    pub fn from_color(rgba: [u8; 4]) -> Self {
        let dark = |v: u8| (v as f32 * 0.7) as u8;
        Self {
            angle: 180.0,
            stops: vec![
                (rgba, 0.0),
                ([dark(rgba[0]), dark(rgba[1]), dark(rgba[2]), rgba[3]], 1.0),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gradients_round_trip() {
        let g = Linear::parse("@linear-gradient(90deg, #3b82f6 0%, #8b5cf6 100%)").unwrap();
        assert_eq!(g.angle, 90.0);
        assert_eq!(g.stops.len(), 2);
        assert_eq!(g.stops[1].0, [0x8b, 0x5c, 0xf6, 255]);
        assert_eq!(
            g.to_source(),
            "@linear-gradient(90deg, #3b82f6 0%, #8b5cf6 100%)"
        );
        let even = Linear::parse("@linear-gradient(180deg, #000000, #ffffff80)").unwrap();
        assert_eq!(even.stops[1].1, 1.0);
        assert_eq!(even.stops[1].0[3], 0x80);
        assert!(Linear::parse("@radial-gradient(circle, #000 0%, #fff 100%)").is_none());
        assert!(Linear::parse("@linear-gradient(90deg, root.c 0%, #fff 100%)").is_none());
        let g = Linear::from_color([100, 200, 50, 255]);
        assert_eq!(
            g.to_source(),
            "@linear-gradient(180deg, #64c832 0%, #468c23 100%)"
        );
    }
}
