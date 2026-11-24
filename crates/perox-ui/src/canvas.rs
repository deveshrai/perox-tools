//! Drawing in Rust for a Canvas (the designer's palette item: an `Image` named, say,
//! `plot`, with `plot_image`, `plot_width`, `plot_height` and `plot_resized()`).
//!
//! Draw with [tiny-skia](https://docs.rs/tiny-skia) (paths, strokes, gradients) on a
//! pixmap the canvas's size in physical pixels, and hand it over:
//!
//! ```ignore
//! fn plot_resized(&mut self, ui: &MainWindow) {
//!     let mut c = perox_ui::canvas::Canvas::new(ui.window(), ui.get_plot_width(), ui.get_plot_height());
//!     c.pixmap.fill(tiny_skia::Color::BLACK);
//!     // … draw in points: c.transform() maps them to pixels …
//!     ui.set_plot_image(c.image());
//! }
//! ```

pub use tiny_skia;

/// A pixmap for a canvas of `width` × `height` points, at the window's scale.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub pixmap: tiny_skia::Pixmap,
    /// Physical pixels per point.
    pub scale: f32,
}

impl Canvas {
    /// At least 1 × 1, so a canvas that's not laid out yet still works.
    pub fn new(window: &slint::Window, width: f32, height: f32) -> Self {
        Self::with_scale(width, height, window.scale_factor())
    }

    pub fn with_scale(width: f32, height: f32, scale: f32) -> Self {
        let px = |v: f32| ((v * scale).round() as u32).clamp(1, 16384);
        Self {
            pixmap: tiny_skia::Pixmap::new(px(width), px(height)).expect("non-zero size"),
            scale,
        }
    }

    /// Width and height in points.
    pub fn size(&self) -> (f32, f32) {
        (
            self.pixmap.width() as f32 / self.scale,
            self.pixmap.height() as f32 / self.scale,
        )
    }

    /// Maps points to the pixmap's pixels: draw with this and think in points.
    pub fn transform(&self) -> tiny_skia::Transform {
        tiny_skia::Transform::from_scale(self.scale, self.scale)
    }

    /// The picture, for the canvas's `<id>_image` property.
    pub fn image(&self) -> slint::Image {
        image(&self.pixmap)
    }
}

/// A tiny-skia pixmap (premultiplied RGBA) as a Slint image.
pub fn image(p: &tiny_skia::Pixmap) -> slint::Image {
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        p.data(),
        p.width(),
        p.height(),
    );
    slint::Image::from_rgba8_premultiplied(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_in_points_and_pixels() {
        let c = Canvas::with_scale(100.0, 50.0, 2.0);
        assert_eq!((c.pixmap.width(), c.pixmap.height()), (200, 100));
        assert_eq!(c.size(), (100.0, 50.0));
        let empty = Canvas::with_scale(0.0, 0.0, 2.0);
        assert_eq!(empty.pixmap.width(), 1);
        let img = c.image();
        assert_eq!(img.size().width, 200);
    }
}
