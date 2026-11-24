//! The opening animation: the anvil, the pour, the name; then the IDE. About three
//! seconds, skipped by any key or click, turned off in View → Opening Animation.

use std::sync::mpsc::Receiver;
use std::time::Instant;

use eframe::egui;
use perox_brand::{Art, Detail, timing};

/// Seconds the finished picture stays, and the fade to the IDE.
const HOLD: f32 = 0.5;
const FADE: f32 = 0.4;
/// Give up on it if drawing the scene takes longer than this (a very slow machine).
const PATIENCE: f32 = 1.5;

pub struct Intro {
    created: Instant,
    ready: Option<Receiver<Art>>,
    art: Option<Art>,
    started: Option<Instant>,
    /// When skipping began (a key or click), to fade out from there.
    leaving: Option<Instant>,
    texture: Option<egui::TextureHandle>,
}

/// The picture as egui wants it (tiny-skia's pixels are premultiplied RGBA too).
pub fn image(p: &perox_brand::tiny_skia::Pixmap) -> egui::ColorImage {
    egui::ColorImage::from_rgba_premultiplied([p.width() as usize, p.height() as usize], p.data())
}

impl Intro {
    /// Starts drawing the scene, `points` across, on a thread.
    pub fn new(ctx: &egui::Context, points: f32) -> Self {
        let px = (points * ctx.pixels_per_point().max(1.0)).round() as u32;
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Art::new(px, Detail::Full));
            ctx2.request_repaint();
        });
        Self {
            created: Instant::now(),
            ready: Some(rx),
            art: None,
            started: None,
            leaving: None,
            texture: None,
        }
    }

    /// Draws it over everything; `false` once it's over.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if let Some(rx) = &self.ready {
            match rx.try_recv() {
                Ok(art) => {
                    self.art = Some(art);
                    self.ready = None;
                    self.started = Some(Instant::now());
                }
                Err(_) if self.created.elapsed().as_secs_f32() > PATIENCE => return false,
                Err(_) => {}
            }
        }
        let skip = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            })
        });
        if skip && self.leaving.is_none() {
            self.leaving = Some(Instant::now());
        }
        let t = self.started.map_or(0.0, |s| s.elapsed().as_secs_f32());
        let fade_from = timing::SECONDS + HOLD;
        let out = match self.leaving {
            Some(l) => l.elapsed().as_secs_f32() / FADE,
            None => (t - fade_from) / FADE,
        }
        .clamp(0.0, 1.0);
        if out >= 1.0 {
            return false;
        }
        let alpha = 1.0 - out;
        let screen = ctx.content_rect();
        let backdrop = egui::Color32::from_rgb(20, 21, 24).gamma_multiply(alpha);
        egui::Area::new(egui::Id::unique("intro"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                // Covers the IDE and takes its clicks while it shows.
                let (rect, _) =
                    ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                ui.painter().rect_filled(rect, 0.0, backdrop);
                let Some(art) = &self.art else { return };
                let frame = art.frame((t / timing::SECONDS).min(1.0));
                let img = image(&frame);
                let tex = match &mut self.texture {
                    Some(tx) => {
                        tx.set(img, egui::TextureOptions::LINEAR);
                        tx.clone()
                    }
                    None => {
                        let tx = ctx.load_texture("intro", img, egui::TextureOptions::LINEAR);
                        self.texture = Some(tx.clone());
                        tx
                    }
                };
                let side = art.size() as f32 / ctx.pixels_per_point();
                let centre = rect.center() - egui::vec2(0.0, 40.0);
                let r = egui::Rect::from_center_size(centre, egui::vec2(side, side));
                ui.painter().image(
                    tex.id(),
                    r,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE.gamma_multiply(alpha),
                );
                // The name comes up under the anvil as the chrome finishes spreading.
                let name = ((t / timing::SECONDS - 0.7) / 0.25).clamp(0.0, 1.0) * alpha;
                if name > 0.0 {
                    let y = r.top() + side * 0.92;
                    ui.painter().text(
                        egui::pos2(centre.x, y),
                        egui::Align2::CENTER_TOP,
                        "PeroxIDE",
                        egui::FontId::proportional(side * 0.085),
                        egui::Color32::from_rgb(226, 230, 234).gamma_multiply(name),
                    );
                }
            });
        ctx.request_repaint();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, intro: &mut Intro, events: Vec<egui::Event>) -> bool {
        let mut shown = true;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| shown = intro.show(ui.ctx()));
        out.textures_delta.clear();
        shown
    }

    #[test]
    fn plays_and_a_key_skips_it() {
        let ctx = egui::Context::default();
        let mut intro = Intro::new(&ctx, 200.0);
        let begun = Instant::now();
        while intro.art.is_none() {
            assert!(
                frame(&ctx, &mut intro, vec![]),
                "gave up before the scene was ready"
            );
            assert!(begun.elapsed().as_secs_f32() < PATIENCE);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // It draws a frame (the texture exists) and keeps going.
        assert!(frame(&ctx, &mut intro, vec![]));
        assert!(intro.texture.is_some());
        let key = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        assert!(frame(&ctx, &mut intro, vec![key]));
        std::thread::sleep(std::time::Duration::from_secs_f32(FADE + 0.05));
        assert!(
            !frame(&ctx, &mut intro, vec![]),
            "still showing after the fade"
        );
    }
}
