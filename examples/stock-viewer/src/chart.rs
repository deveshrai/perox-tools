//! The price chart: closing prices as SVG path commands for the window's `Path`
//! element, whose view box is [`WIDTH`] × [`HEIGHT`] (set in `ui/main_window.slint`).

/// The chart's coordinate space; the `Path` stretches it over its area.
pub const WIDTH: f64 = 1000.0;
pub const HEIGHT: f64 = 100.0;

/// A line through `values`, oldest on the left, highest at the top, with a little room
/// above and below. Empty when there are fewer than two values.
pub fn line_commands(values: &[f64]) -> String {
    let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if values.len() < 2 {
        return String::new();
    }
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = hi - lo;
    let margin = HEIGHT * 0.06;
    let step = WIDTH / (values.len() - 1) as f64;
    let mut out = String::with_capacity(values.len() * 14);
    for (i, v) in values.iter().enumerate() {
        let x = i as f64 * step;
        // A flat series (no movement over the range) runs through the middle.
        let y = if hi > lo {
            margin + (hi - v) / span * (HEIGHT - 2.0 * margin)
        } else {
            HEIGHT / 2.0
        };
        out.push_str(if i == 0 { "M " } else { " L " });
        out.push_str(&format!("{x:.1} {y:.2}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_the_box_with_the_high_at_the_top() {
        let c = line_commands(&[10.0, 20.0, 15.0]);
        assert_eq!(c, "M 0.0 94.00 L 500.0 6.00 L 1000.0 50.00");
    }

    #[test]
    fn flat_and_short_series() {
        assert_eq!(line_commands(&[5.0]), "");
        assert_eq!(line_commands(&[]), "");
        assert_eq!(line_commands(&[3.0, 3.0]), "M 0.0 50.00 L 1000.0 50.00");
        // Gaps (NaN) are left out rather than breaking the path.
        assert_eq!(
            line_commands(&[1.0, f64::NAN, 2.0]),
            "M 0.0 94.00 L 1000.0 6.00"
        );
    }
}
