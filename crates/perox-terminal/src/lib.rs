//! The integrated terminal: alacritty_terminal does the emulation and the PTY (macOS,
//! Linux, Windows ConPTY); this crate turns egui input into terminal bytes and draws the
//! cell grid.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use eframe::egui::{
    self, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, TextFormat, Vec2, text::LayoutJob,
};

const FONT_SIZE: f32 = 13.0;
/// Space between the panel edge and the text.
const PADDING: f32 = 6.0;

#[derive(Clone)]
struct Listener {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// One terminal session running a shell.
pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    events: Receiver<Event>,
    size: Size,
    /// The title the shell set, if any.
    pub title: String,
    /// The shell has exited.
    pub exited: bool,
}

impl std::fmt::Debug for Terminal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Terminal")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl Terminal {
    /// Starts the user's shell in `cwd` with extra environment variables.
    pub fn spawn(
        cwd: &Path,
        env: &[(String, String)],
        ctx: egui::Context,
    ) -> std::io::Result<Self> {
        let size = Size {
            cols: 100,
            rows: 24,
        };
        let (tx, events) = mpsc::channel();
        let listener = Listener { tx, ctx };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &size,
            listener.clone(),
        )));
        let mut vars: HashMap<String, String> = env.iter().cloned().collect();
        vars.insert("TERM".into(), "xterm-256color".into());
        vars.insert("COLORTERM".into(), "truecolor".into());
        let options = tty::Options {
            working_directory: Some(cwd.to_owned()),
            env: vars,
            ..Default::default()
        };
        let pty = tty::new(&options, window_size(size, Vec2::new(8.0, 16.0)), 0)?;
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        Ok(Self {
            term,
            sender,
            events,
            size,
            title: "Terminal".into(),
            exited: false,
        })
    }

    /// Sends bytes to the shell, as if typed.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    /// The visible text, one line per row (for agents and tests).
    pub fn screen_text(&self) -> String {
        let term = self.term.lock();
        let content = term.renderable_content();
        let mut rows: Vec<String> = vec![String::new(); self.size.rows];
        for cell in content.display_iter {
            let row = (cell.point.line.0 + content.display_offset as i32) as usize;
            if let Some(r) = rows.get_mut(row)
                && !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
            {
                r.push(cell.c);
            }
        }
        rows.iter()
            .map(|r| r.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_owned()
    }

    fn handle_events(&mut self) {
        for e in self.events.try_iter() {
            match e {
                Event::Title(t) => self.title = t,
                Event::ResetTitle => self.title = "Terminal".into(),
                Event::Exit | Event::ChildExit(_) => self.exited = true,
                Event::PtyWrite(s) => {
                    let _ = self.sender.send(Msg::Input(s.into_bytes().into()));
                }
                _ => {}
            }
        }
    }

    /// Draws the terminal and handles input while it has focus.
    pub fn show(&mut self, ui: &mut egui::Ui, id_salt: impl std::hash::Hash + std::fmt::Debug) {
        self.handle_events();
        let font = FontId::monospace(FONT_SIZE);
        let (line_h, cell_w) = ui
            .ctx()
            .fonts_mut(|f| (f.row_height(&font), f.glyph_width(&font, 'M')));
        let avail = ui.available_size();
        let inner = avail - Vec2::splat(2.0 * PADDING);
        let size = Size {
            cols: ((inner.x / cell_w).floor() as usize).max(10),
            rows: ((inner.y / line_h).floor() as usize).max(2),
        };
        if size != self.size {
            self.size = size;
            self.term.lock().resize(size);
            let _ = self
                .sender
                .send(Msg::Resize(window_size(size, Vec2::new(cell_w, line_h))));
        }
        let id = ui.make_persistent_id(id_salt);
        let (rect, resp) = ui.allocate_exact_size(avail, Sense::click());
        if resp.clicked() {
            resp.request_focus();
        }
        let focused = ui.memory(|m| m.has_focus(id)) || resp.has_focus();
        let focus_id = resp.id;
        if ui.memory(|m| m.has_focus(focus_id)) {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    focus_id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
            self.input(ui);
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() >= 1.0 {
                let lines = (scroll / line_h).round() as i32;
                if lines != 0 {
                    self.term.lock().scroll_display(Scroll::Delta(lines));
                }
            }
        }
        self.paint(
            ui,
            rect,
            &font,
            line_h,
            cell_w,
            focused || ui.memory(|m| m.has_focus(focus_id)),
        );
    }

    fn input(&mut self, ui: &egui::Ui) {
        let mode = *self.term.lock().mode();
        let app_cursor = mode.contains(TermMode::APP_CURSOR);
        let mut out: Vec<u8> = Vec::new();
        for event in ui.input(|i| i.events.clone()) {
            match event {
                egui::Event::Text(t) => out.extend_from_slice(t.as_bytes()),
                egui::Event::Paste(t) => {
                    if mode.contains(TermMode::BRACKETED_PASTE) {
                        out.extend_from_slice(b"\x1b[200~");
                        out.extend_from_slice(t.replace('\x1b', "").as_bytes());
                        out.extend_from_slice(b"\x1b[201~");
                    } else {
                        out.extend_from_slice(
                            t.replace("\r\n", "\r").replace('\n', "\r").as_bytes(),
                        );
                    }
                }
                // On Windows and Linux Ctrl+C arrives as Copy: send it to the shell (interrupt).
                egui::Event::Copy if !cfg!(target_os = "macos") => out.push(0x03),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if let Some(bytes) = key_bytes(key, modifiers, app_cursor) {
                        out.extend_from_slice(&bytes);
                    }
                }
                _ => {}
            }
        }
        if !out.is_empty() {
            self.term.lock().scroll_display(Scroll::Bottom);
            self.write(out);
        }
    }

    fn paint(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        font: &FontId,
        line_h: f32,
        cell_w: f32,
        focused: bool,
    ) {
        let painter = ui.painter_at(rect);
        let dark = ui.visuals().dark_mode;
        let palette = Palette::new(dark);
        painter.rect_filled(rect, 0.0, palette.bg);
        let rect = rect.shrink(PADDING);
        let term = self.term.lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let resolve = |c: Color| -> Color32 {
            match c {
                Color::Spec(Rgb { r, g, b }) => Color32::from_rgb(r, g, b),
                Color::Indexed(i) => colors[i as usize].map_or_else(|| palette.indexed(i), to32),
                Color::Named(n) => colors[n as usize].map_or_else(|| palette.named(n), to32),
            }
        };
        let mut rows: Vec<Vec<(char, Color32, Color32, Flags)>> = vec![Vec::new(); self.size.rows];
        for cell in content.display_iter {
            let row = (cell.point.line.0 + content.display_offset as i32) as usize;
            let Some(r) = rows.get_mut(row) else { continue };
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let (mut fg, mut bg) = (resolve(cell.fg), resolve(cell.bg));
            if cell.flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if cell.flags.contains(Flags::DIM) {
                fg = fg.gamma_multiply(0.66);
            }
            r.push((cell.c, fg, bg, cell.flags));
        }
        for (y, row) in rows.iter().enumerate() {
            let top = rect.top() + y as f32 * line_h;
            for (x, (_, _, bg, _)) in row.iter().enumerate() {
                if *bg != palette.bg {
                    let left = rect.left() + x as f32 * cell_w;
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(left, top), Vec2::new(cell_w, line_h)),
                        0.0,
                        *bg,
                    );
                }
            }
            let mut job = LayoutJob::default();
            for (c, fg, _, flags) in row {
                let c = if flags.contains(Flags::HIDDEN) {
                    ' '
                } else {
                    *c
                };
                job.append(
                    &c.to_string(),
                    0.0,
                    TextFormat {
                        font_id: font.clone(),
                        color: *fg,
                        italics: flags.contains(Flags::ITALIC),
                        underline: if flags.intersects(Flags::ALL_UNDERLINES) {
                            Stroke::new(1.0, *fg)
                        } else {
                            Stroke::NONE
                        },
                        strikethrough: if flags.contains(Flags::STRIKEOUT) {
                            Stroke::new(1.0, *fg)
                        } else {
                            Stroke::NONE
                        },
                        ..Default::default()
                    },
                );
            }
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(Pos2::new(rect.left(), top), galley, palette.fg);
        }
        // Cursor, when scrolled to the bottom.
        if content.display_offset == 0 {
            let p = content.cursor.point;
            let pos = Pos2::new(
                rect.left() + p.column.0 as f32 * cell_w,
                rect.top() + p.line.0 as f32 * line_h,
            );
            let r = Rect::from_min_size(pos, Vec2::new(cell_w, line_h));
            if focused {
                painter.rect_filled(r, 0.0, palette.cursor.gamma_multiply(0.7));
            } else {
                painter.rect_stroke(
                    r,
                    0.0,
                    Stroke::new(1.0, palette.cursor),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
    }
}

fn window_size(size: Size, cell: Vec2) -> WindowSize {
    WindowSize {
        num_lines: size.rows as u16,
        num_cols: size.cols as u16,
        cell_width: cell.x as u16,
        cell_height: cell.y as u16,
    }
}

fn to32(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Bytes a key sends to the shell (xterm conventions).
fn key_bytes(key: Key, m: egui::Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    let csi = |s: &str| Some(format!("\x1b[{s}").into_bytes());
    let arrow = |c: char| {
        if m.shift || m.alt || m.ctrl {
            // xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4)
            let p = 1 + u8::from(m.shift) + 2 * u8::from(m.alt) + 4 * u8::from(m.ctrl);
            Some(format!("\x1b[1;{p}{c}").into_bytes())
        } else if app_cursor {
            Some(format!("\x1bO{c}").into_bytes())
        } else {
            Some(format!("\x1b[{c}").into_bytes())
        }
    };
    // Ctrl+letter: control characters (Ctrl+C = 0x03). Not ⌘ on macOS.
    if m.ctrl && !m.mac_cmd {
        let name = key.name();
        if name.len() == 1 {
            let ch = name.as_bytes()[0].to_ascii_lowercase();
            if ch.is_ascii_lowercase() {
                return Some(vec![ch - b'a' + 1]);
            }
        }
    }
    match key {
        Key::Enter => Some(b"\r".to_vec()),
        Key::Backspace => Some(if m.alt {
            b"\x1b\x7f".to_vec()
        } else {
            vec![0x7f]
        }),
        Key::Tab if m.shift => csi("Z"),
        Key::Tab => Some(b"\t".to_vec()),
        Key::Escape => Some(b"\x1b".to_vec()),
        Key::ArrowUp => arrow('A'),
        Key::ArrowDown => arrow('B'),
        Key::ArrowRight => arrow('C'),
        Key::ArrowLeft => arrow('D'),
        Key::Home => arrow('H'),
        Key::End => arrow('F'),
        Key::Delete => csi("3~"),
        Key::PageUp => csi("5~"),
        Key::PageDown => csi("6~"),
        Key::Insert => csi("2~"),
        _ => None,
    }
}

struct Palette {
    fg: Color32,
    bg: Color32,
    cursor: Color32,
    ansi: [Color32; 16],
}

impl Palette {
    fn new(dark: bool) -> Self {
        let c = |h: u32| Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
        if dark {
            Self {
                fg: c(0xd4d4d4),
                bg: c(0x181818),
                cursor: c(0xaeafad),
                ansi: [
                    c(0x000000),
                    c(0xcd3131),
                    c(0x0dbc79),
                    c(0xe5e510),
                    c(0x2472c8),
                    c(0xbc3fbc),
                    c(0x11a8cd),
                    c(0xe5e5e5),
                    c(0x666666),
                    c(0xf14c4c),
                    c(0x23d18b),
                    c(0xf5f543),
                    c(0x3b8eea),
                    c(0xd670d6),
                    c(0x29b8db),
                    c(0xffffff),
                ],
            }
        } else {
            Self {
                fg: c(0x1f2328),
                bg: c(0xfafafa),
                cursor: c(0x1f2328),
                ansi: [
                    c(0x000000),
                    c(0xcd3131),
                    c(0x00bc00),
                    c(0x949800),
                    c(0x0451a5),
                    c(0xbc05bc),
                    c(0x0598bc),
                    c(0x555555),
                    c(0x666666),
                    c(0xcd3131),
                    c(0x14ce14),
                    c(0xb5ba00),
                    c(0x0451a5),
                    c(0xbc05bc),
                    c(0x0598bc),
                    c(0xa5a5a5),
                ],
            }
        }
    }

    fn named(&self, n: NamedColor) -> Color32 {
        match n {
            NamedColor::Foreground | NamedColor::BrightForeground => self.fg,
            NamedColor::DimForeground => self.fg.gamma_multiply(0.66),
            NamedColor::Background => self.bg,
            NamedColor::Cursor => self.cursor,
            other => {
                let i = other as usize;
                if i < 16 {
                    self.ansi[i]
                } else {
                    // Dim variants follow the 16 colours in order.
                    let d = i.saturating_sub(NamedColor::DimBlack as usize).min(7);
                    self.ansi[d].gamma_multiply(0.66)
                }
            }
        }
    }

    /// The 256-colour palette: 16 ANSI, a 6×6×6 cube, then 24 greys.
    fn indexed(&self, i: u8) -> Color32 {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
            }
            _ => {
                let v = 8 + (i - 232) * 10;
                Color32::from_rgb(v, v, v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_to_bytes() {
        let none = egui::Modifiers::NONE;
        assert_eq!(
            key_bytes(Key::ArrowUp, none, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_bytes(Key::ArrowUp, none, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(Key::C, egui::Modifiers::CTRL, false),
            Some(vec![3])
        );
        assert_eq!(key_bytes(Key::Enter, none, false), Some(b"\r".to_vec()));
        assert_eq!(
            key_bytes(Key::ArrowLeft, egui::Modifiers::ALT, false),
            Some(b"\x1b[1;3D".to_vec())
        );
    }

    #[test]
    fn palette_cube_and_greys() {
        let p = Palette::new(true);
        assert_eq!(p.indexed(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(p.indexed(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(p.indexed(232), Color32::from_rgb(8, 8, 8));
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_shell_command() {
        let ctx = egui::Context::default();
        let t = Terminal::spawn(&std::env::temp_dir(), &[], ctx).expect("pty");
        t.write(b"echo perox-$((40+2))\r".to_vec());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if t.screen_text().contains("perox-42") {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("shell output never appeared:\n{}", t.screen_text());
    }
}
