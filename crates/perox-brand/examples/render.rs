//! Renders the icon at every size, and frames of the animation, to a folder:
//! `cargo run -p perox-brand --release --example render -- assets/brand`.

use perox_brand::tiny_skia::{Pixmap, PixmapPaint, Transform};

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&out).expect("out dir");
    for px in [16, 32, 64, 128, 256, 512, 1024] {
        perox_brand::icon(px)
            .save_png(out.join(format!("icon-{px}.png")))
            .expect("png");
    }
    // The animation as a strip of six frames on the intro's backdrop.
    let side = 320;
    let art = perox_brand::Art::new(side, perox_brand::Detail::Full);
    let n = 6;
    let mut strip = Pixmap::new(side * n, side).expect("strip");
    strip.fill(perox_brand::tiny_skia::Color::from_rgba8(20, 21, 24, 255));
    for k in 0..n {
        let f = art.frame(k as f32 / (n - 1) as f32);
        strip.draw_pixmap(
            (k * side) as i32,
            0,
            f.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    strip.save_png(out.join("intro-frames.png")).expect("png");
}
