//! The design canvas's picture: the `.slint` source compiled by `slint-interpreter` and
//! drawn by Slint's software renderer into a texture, so the designer shows exactly what
//! the app will. Element rectangles come from the interpreter too (the API Slint's own
//! live preview uses), keyed by each element's source offset.
//!
//! Slint keeps its platform per thread; the first `Preview` on a thread installs a
//! minimal one that hands out software-rendered windows.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use eframe::egui::{self, ColorImage, Rect, TextureHandle, TextureOptions, Vec2, pos2, vec2};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType, TargetPixel,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint_interpreter::{ComponentHandle, ComponentInstance};

thread_local! {
    /// The window the platform created last; `Preview` takes it right after `create`.
    static CREATED: RefCell<Option<Rc<MinimalSoftwareWindow>>> = const { RefCell::new(None) };
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct DesignPlatform;

impl Platform for DesignPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        CREATED.with(|c| *c.borrow_mut() = Some(w.clone()));
        Ok(w)
    }
}

fn install_platform() {
    INSTALLED.with(|done| {
        if !done.get() {
            // Fails if this thread already has a platform (only ours sets one here).
            let _ = slint::platform::set_platform(Box::new(DesignPlatform));
            done.set(true);
        }
    });
}

/// The widget style apps get on this OS by default (Slint's "native").
pub fn native_style() -> &'static str {
    if cfg!(target_os = "macos") || cfg!(target_os = "ios") {
        "cupertino"
    } else if cfg!(target_os = "android") {
        "material"
    } else {
        "fluent"
    }
}

pub const STYLES: &[&str] = &["fluent", "cupertino", "material", "cosmic"];

#[derive(Default)]
pub struct Preview {
    /// What the current instance was built from.
    built: Option<(u64, String)>,
    instance: Option<ComponentInstance>,
    window: Option<Rc<MinimalSoftwareWindow>>,
    /// Errors from the last compile (the previous picture stays up).
    pub diagnostics: Vec<String>,
    texture: Option<TextureHandle>,
    /// Size and scale of the last render.
    rendered: Option<(Vec2, f32)>,
    path: PathBuf,
}

impl std::fmt::Debug for Preview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preview")
            .field("built", &self.built.as_ref().map(|(_, c)| c))
            .field("diagnostics", &self.diagnostics)
            .finish_non_exhaustive()
    }
}

fn hash(parts: &[&str]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

impl Preview {
    /// Rebuilds the component if the source, component or style changed. `path` is the
    /// file's location (for its imports); it needn't exist.
    pub fn update(&mut self, source: &str, path: &Path, component: &str, style: &str) {
        let key = hash(&[source, component, style, &path.to_string_lossy()]);
        if self.built.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        self.built = Some((key, component.to_owned()));
        install_platform();
        let mut compiler = slint_interpreter::Compiler::default();
        compiler.set_style(style.to_owned());
        compiler.set_edition((1, 19));
        let result =
            spin_on::spin_on(compiler.build_from_source(source.to_owned(), path.to_owned()));
        self.diagnostics = result
            .diagnostics()
            .filter(|d| d.level() == slint_interpreter::DiagnosticLevel::Error)
            .map(|d| {
                let (line, col) = d.line_column();
                format!("{line}:{col}: {}", d.message())
            })
            .collect();
        let Some(def) = result.component(component) else {
            if self.diagnostics.is_empty() {
                self.diagnostics
                    .push(format!("`{component}` must be exported to show it"));
            }
            return;
        };
        if !self.diagnostics.is_empty() {
            return; // keep the last good picture
        }
        CREATED.with(|c| c.borrow_mut().take());
        let Ok(instance) = def.create() else {
            self.diagnostics
                .push("couldn't create the component".into());
            return;
        };
        let window = CREATED.with(|c| c.borrow_mut().take());
        if let Some(w) = &window {
            // `show` makes the item tree live (layouts, rendering); nothing appears on
            // screen with this platform.
            let _ = instance.show();
            w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
        }
        self.instance = Some(instance);
        self.window = window;
        self.rendered = None;
        self.path = path.to_owned();
    }

    pub fn is_ready(&self) -> bool {
        self.instance.is_some() && self.window.is_some()
    }

    /// The window's natural size (from its layout), in logical points.
    pub fn preferred_size(&self) -> Option<Vec2> {
        let instance = self.instance.as_ref()?;
        let inner = i_slint_core::window::WindowInner::from_pub(instance.window());
        let comp = inner.component();
        let comp = i_slint_core::item_tree::ItemTreeRc::borrow_pin(&comp);
        let h = comp
            .as_ref()
            .layout_info(i_slint_core::layout::Orientation::Horizontal);
        let v = comp
            .as_ref()
            .layout_info(i_slint_core::layout::Orientation::Vertical);
        let w = h.preferred_bounded();
        let ht = v.preferred_bounded();
        (w > 0.0 && ht > 0.0).then(|| vec2(w, ht))
    }

    /// Draws at `size` (points) and `scale` (pixels per point); returns the texture.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        size: Vec2,
        scale: f32,
    ) -> Option<&TextureHandle> {
        let window = self.window.clone()?;
        slint::platform::update_timers_and_animations();
        let px = (
            (size.x * scale).round().max(1.0) as u32,
            (size.y * scale).round().max(1.0) as u32,
        );
        if self.rendered != Some((size, scale)) {
            window.dispatch_event(WindowEvent::ScaleFactorChanged {
                scale_factor: scale,
            });
            window.set_size(slint::PhysicalSize::new(px.0, px.1));
            window.request_redraw();
        }
        let (w, h) = (px.0 as usize, px.1 as usize);
        let mut pixels = Vec::new();
        let drew = window.draw_if_needed(|renderer| {
            pixels = vec![PremultipliedRgbaColor::background(); w * h];
            renderer.render(&mut pixels, w);
        });
        if drew || self.texture.is_none() {
            if pixels.len() == w * h {
                let mut bytes = Vec::with_capacity(w * h * 4);
                for p in &pixels {
                    bytes.extend_from_slice(&[p.red, p.green, p.blue, p.alpha]);
                }
                let image = ColorImage::from_rgba_premultiplied([w, h], &bytes);
                match &mut self.texture {
                    Some(t) => t.set(image, TextureOptions::LINEAR),
                    None => {
                        self.texture =
                            Some(ctx.load_texture("slint-preview", image, TextureOptions::LINEAR))
                    }
                }
            }
            self.rendered = Some((size, scale));
        }
        self.texture.as_ref()
    }

    /// Where the element whose type name starts at `offset` was drawn, relative to the
    /// window's top-left, in points. `None` if it isn't showing.
    pub fn rect_at_offset(&self, offset: usize) -> Option<Rect> {
        let instance = self.instance.as_ref()?;
        let sp = i_slint_compiler::source_path::SourcePath::new(&self.path);
        let rects = instance.component_positions(&sp, offset as u32);
        let r = rects.first()?.rect;
        Some(Rect::from_min_size(
            pos2(r.origin.x, r.origin.y),
            vec2(r.size.width, r.size.height),
        ))
    }
}
