//! PeroxIDE's icons: [Lucide](https://lucide.dev) 0.460.0 SVGs (ISC license), embedded in
//! the binary. They're drawn white and tinted to the theme's text colour at use.
//!
//! The app must install egui's image loaders once (`egui_extras::install_image_loaders`).
//! To add an icon: download it from the same pinned Lucide version into `icons/`, replace
//! `currentColor` with `#ffffff`, and add a variant here.

use eframe::egui::{self, Image, ImageSource, Ui, Vec2};

/// Icon size in points for menus, toolbars and lists.
pub const SIZE: f32 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    ArrowDown,
    ArrowDownToLine,
    ArrowUp,
    CaseSensitive,
    ChevronDown,
    ChevronRight,
    CircleCheck,
    CircleX,
    Command,
    File,
    FileCheck,
    FileCode,
    FileCog,
    FileImage,
    FileSearch,
    FileText,
    FlaskConical,
    Folder,
    FolderOpen,
    Hammer,
    Info,
    LayoutDashboard,
    Lightbulb,
    ListChecks,
    LogOut,
    Paintbrush,
    Play,
    Plug,
    Regex,
    Replace,
    ReplaceAll,
    RotateCcw,
    Save,
    SaveAll,
    SearchCode,
    Settings,
    Settings2,
    ShieldCheck,
    Square,
    TextCursorInput,
    TriangleAlert,
    WholeWord,
    X,
}

impl Icon {
    pub const ALL: &[Icon] = &[
        Icon::ArrowDown,
        Icon::ArrowDownToLine,
        Icon::ArrowUp,
        Icon::CaseSensitive,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::CircleCheck,
        Icon::CircleX,
        Icon::Command,
        Icon::File,
        Icon::FileCheck,
        Icon::FileCode,
        Icon::FileCog,
        Icon::FileImage,
        Icon::FileSearch,
        Icon::FileText,
        Icon::FlaskConical,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::Hammer,
        Icon::Info,
        Icon::LayoutDashboard,
        Icon::Lightbulb,
        Icon::ListChecks,
        Icon::LogOut,
        Icon::Paintbrush,
        Icon::Play,
        Icon::Plug,
        Icon::Regex,
        Icon::Replace,
        Icon::ReplaceAll,
        Icon::RotateCcw,
        Icon::Save,
        Icon::SaveAll,
        Icon::SearchCode,
        Icon::Settings,
        Icon::Settings2,
        Icon::ShieldCheck,
        Icon::Square,
        Icon::TextCursorInput,
        Icon::TriangleAlert,
        Icon::WholeWord,
        Icon::X,
    ];

    pub fn source(self) -> ImageSource<'static> {
        match self {
            Icon::ArrowDown => egui::include_image!("../icons/arrow-down.svg"),
            Icon::ArrowDownToLine => egui::include_image!("../icons/arrow-down-to-line.svg"),
            Icon::ArrowUp => egui::include_image!("../icons/arrow-up.svg"),
            Icon::CaseSensitive => egui::include_image!("../icons/case-sensitive.svg"),
            Icon::ChevronDown => egui::include_image!("../icons/chevron-down.svg"),
            Icon::ChevronRight => egui::include_image!("../icons/chevron-right.svg"),
            Icon::CircleCheck => egui::include_image!("../icons/circle-check.svg"),
            Icon::CircleX => egui::include_image!("../icons/circle-x.svg"),
            Icon::Command => egui::include_image!("../icons/command.svg"),
            Icon::File => egui::include_image!("../icons/file.svg"),
            Icon::FileCheck => egui::include_image!("../icons/file-check.svg"),
            Icon::FileCode => egui::include_image!("../icons/file-code.svg"),
            Icon::FileCog => egui::include_image!("../icons/file-cog.svg"),
            Icon::FileImage => egui::include_image!("../icons/file-image.svg"),
            Icon::FileSearch => egui::include_image!("../icons/file-search.svg"),
            Icon::FileText => egui::include_image!("../icons/file-text.svg"),
            Icon::FlaskConical => egui::include_image!("../icons/flask-conical.svg"),
            Icon::Folder => egui::include_image!("../icons/folder.svg"),
            Icon::FolderOpen => egui::include_image!("../icons/folder-open.svg"),
            Icon::Hammer => egui::include_image!("../icons/hammer.svg"),
            Icon::Info => egui::include_image!("../icons/info.svg"),
            Icon::LayoutDashboard => egui::include_image!("../icons/layout-dashboard.svg"),
            Icon::Lightbulb => egui::include_image!("../icons/lightbulb.svg"),
            Icon::ListChecks => egui::include_image!("../icons/list-checks.svg"),
            Icon::LogOut => egui::include_image!("../icons/log-out.svg"),
            Icon::Paintbrush => egui::include_image!("../icons/paintbrush.svg"),
            Icon::Play => egui::include_image!("../icons/play.svg"),
            Icon::Plug => egui::include_image!("../icons/plug.svg"),
            Icon::Regex => egui::include_image!("../icons/regex.svg"),
            Icon::Replace => egui::include_image!("../icons/replace.svg"),
            Icon::ReplaceAll => egui::include_image!("../icons/replace-all.svg"),
            Icon::RotateCcw => egui::include_image!("../icons/rotate-ccw.svg"),
            Icon::Save => egui::include_image!("../icons/save.svg"),
            Icon::SaveAll => egui::include_image!("../icons/save-all.svg"),
            Icon::SearchCode => egui::include_image!("../icons/search-code.svg"),
            Icon::Settings => egui::include_image!("../icons/settings.svg"),
            Icon::Settings2 => egui::include_image!("../icons/settings-2.svg"),
            Icon::ShieldCheck => egui::include_image!("../icons/shield-check.svg"),
            Icon::Square => egui::include_image!("../icons/square.svg"),
            Icon::TextCursorInput => egui::include_image!("../icons/text-cursor-input.svg"),
            Icon::TriangleAlert => egui::include_image!("../icons/triangle-alert.svg"),
            Icon::WholeWord => egui::include_image!("../icons/whole-word.svg"),
            Icon::X => egui::include_image!("../icons/x.svg"),
        }
    }

    /// The icon at the standard size, tinted with the current text colour.
    pub fn image(self, ui: &Ui) -> Image<'static> {
        self.sized(ui, SIZE)
    }

    pub fn sized(self, ui: &Ui, size: f32) -> Image<'static> {
        Image::new(self.source())
            .fit_to_exact_size(Vec2::splat(size))
            .tint(ui.visuals().text_color())
    }

    /// The icon in a specific colour (status icons: errors, warnings).
    pub fn colored(self, color: egui::Color32) -> Image<'static> {
        Image::new(self.source())
            .fit_to_exact_size(Vec2::splat(SIZE))
            .tint(color)
    }
}

/// A menu entry or button with an icon, text and an optional shortcut.
pub fn button(ui: &Ui, icon: Icon, text: &str, shortcut: &str) -> egui::Button<'static> {
    egui::Button::image_and_text(icon.image(ui), text.to_owned()).shortcut_text(shortcut.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_white_svg() {
        for icon in Icon::ALL {
            let ImageSource::Bytes { bytes, .. } = icon.source() else {
                panic!("{icon:?} isn't embedded")
            };
            let text = std::str::from_utf8(&bytes).unwrap();
            assert!(
                text.contains("<svg") && text.contains("#ffffff") && !text.contains("currentColor"),
                "{icon:?}"
            );
        }
    }
}
