//! The price chart: closing prices as SVG path commands for the window's two `Path`
//! elements, whose view box is [`WIDTH`] × [`HEIGHT`] (set in `ui/main_window.slint`):
//! the line, and the area under it that the gradient fills.

/// The chart's coordinate space; the paths stretch it over the chart's area.
pub const WIDTH: f64 = 1000.0;
pub const HEIGHT: f64 = 100.0;

/// Where each value goes: x across the box, y with the highest near the top and a little
/// room above and below. Non-finite values are left out.
fn points(values: &[f64]) -> Vec<(f64, f64)> {
    let values: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if values.len() < 2 {
        return Vec::new();
    }
    let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let margin = HEIGHT * 0.08;
    let step = WIDTH / (values.len() - 1) as f64;
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            // A flat series (no movement over the range) runs through the middle.
            let y = if hi > lo {
                margin + (hi - v) / (hi - lo) * (HEIGHT - 2.0 * margin)
            } else {
                HEIGHT / 2.0
            };
            (i as f64 * step, y)
        })
        .collect()
}

/// A line through `values`, oldest on the left. Empty for fewer than two values.
pub fn line_commands(values: &[f64]) -> String {
    points(values)
        .iter()
        .enumerate()
        .map(|(i, (x, y))| format!("{} {x:.1} {y:.2}", if i == 0 { "M" } else { "L" }))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The same line closed along the bottom: the area under it.
pub fn area_commands(values: &[f64]) -> String {
    let line = line_commands(values);
    if line.is_empty() {
        return line;
    }
    format!("{line} L {WIDTH:.1} {HEIGHT:.2} L 0.0 {HEIGHT:.2} Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_the_box_with_the_high_at_the_top() {
        assert_eq!(
            line_commands(&[10.0, 20.0, 15.0]),
            "M 0.0 92.00 L 500.0 8.00 L 1000.0 50.00"
        );
        assert_eq!(
            area_commands(&[10.0, 20.0, 15.0]),
            "M 0.0 92.00 L 500.0 8.00 L 1000.0 50.00 L 1000.0 100.00 L 0.0 100.00 Z"
        );
    }

    #[test]
    fn flat_and_short_series() {
        assert_eq!(line_commands(&[5.0]), "");
        assert_eq!(area_commands(&[]), "");
        assert_eq!(line_commands(&[3.0, 3.0]), "M 0.0 50.00 L 1000.0 50.00");
        assert_eq!(
            line_commands(&[1.0, f64::NAN, 2.0]),
            "M 0.0 92.00 L 1000.0 8.00"
        );
    }
}
