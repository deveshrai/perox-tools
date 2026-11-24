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
    AppWindow,
    ArrowDown,
    ArrowDownToLine,
    ArrowRight,
    ArrowUp,
    CaseSensitive,
    ChevronDown,
    ChevronRight,
    CircleCheck,
    CircleDot,
    CircleX,
    Code,
    Columns3,
    Command,
    Copy,
    File,
    FileCheck,
    FileCode,
    FileCog,
    FileDiff,
    FileImage,
    FileSearch,
    FileText,
    FlaskConical,
    Folder,
    FolderOpen,
    Gauge,
    GitBranch,
    GitCommitHorizontal,
    GitCompare,
    Grid3x3,
    Group,
    Hammer,
    Hash,
    History,
    Image,
    Info,
    LayoutDashboard,
    Lightbulb,
    List,
    ListChecks,
    LogOut,
    Magnet,
    Minus,
    Paintbrush,
    PanelTop,
    PencilRuler,
    Play,
    Plug,
    Plus,
    Puzzle,
    RectangleHorizontal,
    Redo2,
    Regex,
    Replace,
    ReplaceAll,
    RotateCcw,
    Rows3,
    Save,
    SaveAll,
    SearchCode,
    SeparatorHorizontal,
    Settings,
    Settings2,
    ShieldCheck,
    SlidersHorizontal,
    Space,
    Square,
    SquareCheck,
    SquareChevronDown,
    SquareDashed,
    Terminal,
    TextCursorInput,
    Trash2,
    TriangleAlert,
    Type,
    Undo2,
    Ungroup,
    WholeWord,
    X,
    LetterText,
    LoaderCircle,
    MessageSquareWarning,
    ScrollText,
    SquareMousePointer,
    ToggleLeft,
    Moon,
    Sun,
    AlignStartVertical,
    AlignCenterVertical,
    AlignEndVertical,
    AlignStartHorizontal,
    AlignCenterHorizontal,
    AlignEndHorizontal,
    AlignHorizontalDistributeCenter,
    AlignVerticalDistributeCenter,
    ClipboardPaste,
    Scissors,
    Zap,
}

impl Icon {
    pub const ALL: &[Icon] = &[
        Icon::AppWindow,
        Icon::ArrowDown,
        Icon::ArrowDownToLine,
        Icon::ArrowRight,
        Icon::ArrowUp,
        Icon::CaseSensitive,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::CircleCheck,
        Icon::CircleDot,
        Icon::CircleX,
        Icon::Code,
        Icon::Columns3,
        Icon::Command,
        Icon::Copy,
        Icon::File,
        Icon::FileCheck,
        Icon::FileCode,
        Icon::FileCog,
        Icon::FileDiff,
        Icon::FileImage,
        Icon::FileSearch,
        Icon::FileText,
        Icon::FlaskConical,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::Gauge,
        Icon::GitBranch,
        Icon::GitCommitHorizontal,
        Icon::GitCompare,
        Icon::Grid3x3,
        Icon::Group,
        Icon::Hammer,
        Icon::Hash,
        Icon::History,
        Icon::Image,
        Icon::Info,
        Icon::LayoutDashboard,
        Icon::Lightbulb,
        Icon::List,
        Icon::ListChecks,
        Icon::LogOut,
        Icon::Magnet,
        Icon::Minus,
        Icon::Paintbrush,
        Icon::PanelTop,
        Icon::PencilRuler,
        Icon::Play,
        Icon::Plug,
        Icon::Plus,
        Icon::Puzzle,
        Icon::RectangleHorizontal,
        Icon::Redo2,
        Icon::Regex,
        Icon::Replace,
        Icon::ReplaceAll,
        Icon::RotateCcw,
        Icon::Rows3,
        Icon::Save,
        Icon::SaveAll,
        Icon::SearchCode,
        Icon::SeparatorHorizontal,
        Icon::Settings,
        Icon::Settings2,
        Icon::ShieldCheck,
        Icon::SlidersHorizontal,
        Icon::Space,
        Icon::Square,
        Icon::SquareCheck,
        Icon::SquareChevronDown,
        Icon::SquareDashed,
        Icon::Terminal,
        Icon::TextCursorInput,
        Icon::Trash2,
        Icon::TriangleAlert,
        Icon::Type,
        Icon::Undo2,
        Icon::Ungroup,
        Icon::WholeWord,
        Icon::X,
        Icon::LetterText,
        Icon::LoaderCircle,
        Icon::MessageSquareWarning,
        Icon::ScrollText,
        Icon::SquareMousePointer,
        Icon::ToggleLeft,
        Icon::Moon,
        Icon::Sun,
        Icon::AlignStartVertical,
        Icon::AlignCenterVertical,
        Icon::AlignEndVertical,
        Icon::AlignStartHorizontal,
        Icon::AlignCenterHorizontal,
        Icon::AlignEndHorizontal,
        Icon::AlignHorizontalDistributeCenter,
        Icon::AlignVerticalDistributeCenter,
        Icon::ClipboardPaste,
        Icon::Scissors,
        Icon::Zap,
    ];

    pub fn source(self) -> ImageSource<'static> {
        match self {
            Icon::AppWindow => egui::include_image!("../icons/app-window.svg"),
            Icon::ArrowDown => egui::include_image!("../icons/arrow-down.svg"),
            Icon::ArrowDownToLine => egui::include_image!("../icons/arrow-down-to-line.svg"),
            Icon::ArrowRight => egui::include_image!("../icons/arrow-right.svg"),
            Icon::ArrowUp => egui::include_image!("../icons/arrow-up.svg"),
            Icon::CaseSensitive => egui::include_image!("../icons/case-sensitive.svg"),
            Icon::ChevronDown => egui::include_image!("../icons/chevron-down.svg"),
            Icon::ChevronRight => egui::include_image!("../icons/chevron-right.svg"),
            Icon::CircleCheck => egui::include_image!("../icons/circle-check.svg"),
            Icon::CircleDot => egui::include_image!("../icons/circle-dot.svg"),
            Icon::CircleX => egui::include_image!("../icons/circle-x.svg"),
            Icon::Code => egui::include_image!("../icons/code.svg"),
            Icon::Columns3 => egui::include_image!("../icons/columns-3.svg"),
            Icon::Command => egui::include_image!("../icons/command.svg"),
            Icon::Copy => egui::include_image!("../icons/copy.svg"),
            Icon::File => egui::include_image!("../icons/file.svg"),
            Icon::FileCheck => egui::include_image!("../icons/file-check.svg"),
            Icon::FileCode => egui::include_image!("../icons/file-code.svg"),
            Icon::FileCog => egui::include_image!("../icons/file-cog.svg"),
            Icon::FileDiff => egui::include_image!("../icons/file-diff.svg"),
            Icon::FileImage => egui::include_image!("../icons/file-image.svg"),
            Icon::FileSearch => egui::include_image!("../icons/file-search.svg"),
            Icon::FileText => egui::include_image!("../icons/file-text.svg"),
            Icon::FlaskConical => egui::include_image!("../icons/flask-conical.svg"),
            Icon::Folder => egui::include_image!("../icons/folder.svg"),
            Icon::FolderOpen => egui::include_image!("../icons/folder-open.svg"),
            Icon::Gauge => egui::include_image!("../icons/gauge.svg"),
            Icon::GitBranch => egui::include_image!("../icons/git-branch.svg"),
            Icon::GitCommitHorizontal => egui::include_image!("../icons/git-commit-horizontal.svg"),
            Icon::GitCompare => egui::include_image!("../icons/git-compare.svg"),
            Icon::Grid3x3 => egui::include_image!("../icons/grid-3x3.svg"),
            Icon::Group => egui::include_image!("../icons/group.svg"),
            Icon::Hammer => egui::include_image!("../icons/hammer.svg"),
            Icon::Hash => egui::include_image!("../icons/hash.svg"),
            Icon::History => egui::include_image!("../icons/history.svg"),
            Icon::Image => egui::include_image!("../icons/image.svg"),
            Icon::Info => egui::include_image!("../icons/info.svg"),
            Icon::LayoutDashboard => egui::include_image!("../icons/layout-dashboard.svg"),
            Icon::Lightbulb => egui::include_image!("../icons/lightbulb.svg"),
            Icon::List => egui::include_image!("../icons/list.svg"),
            Icon::ListChecks => egui::include_image!("../icons/list-checks.svg"),
            Icon::LogOut => egui::include_image!("../icons/log-out.svg"),
            Icon::Magnet => egui::include_image!("../icons/magnet.svg"),
            Icon::Minus => egui::include_image!("../icons/minus.svg"),
            Icon::Paintbrush => egui::include_image!("../icons/paintbrush.svg"),
            Icon::PanelTop => egui::include_image!("../icons/panel-top.svg"),
            Icon::PencilRuler => egui::include_image!("../icons/pencil-ruler.svg"),
            Icon::Play => egui::include_image!("../icons/play.svg"),
            Icon::Plug => egui::include_image!("../icons/plug.svg"),
            Icon::Plus => egui::include_image!("../icons/plus.svg"),
            Icon::Puzzle => egui::include_image!("../icons/puzzle.svg"),
            Icon::RectangleHorizontal => egui::include_image!("../icons/rectangle-horizontal.svg"),
            Icon::Redo2 => egui::include_image!("../icons/redo-2.svg"),
            Icon::Regex => egui::include_image!("../icons/regex.svg"),
            Icon::Replace => egui::include_image!("../icons/replace.svg"),
            Icon::ReplaceAll => egui::include_image!("../icons/replace-all.svg"),
            Icon::RotateCcw => egui::include_image!("../icons/rotate-ccw.svg"),
            Icon::Rows3 => egui::include_image!("../icons/rows-3.svg"),
            Icon::Save => egui::include_image!("../icons/save.svg"),
            Icon::SaveAll => egui::include_image!("../icons/save-all.svg"),
            Icon::SearchCode => egui::include_image!("../icons/search-code.svg"),
            Icon::SeparatorHorizontal => egui::include_image!("../icons/separator-horizontal.svg"),
            Icon::Settings => egui::include_image!("../icons/settings.svg"),
            Icon::Settings2 => egui::include_image!("../icons/settings-2.svg"),
            Icon::ShieldCheck => egui::include_image!("../icons/shield-check.svg"),
            Icon::SlidersHorizontal => egui::include_image!("../icons/sliders-horizontal.svg"),
            Icon::Space => egui::include_image!("../icons/space.svg"),
            Icon::Square => egui::include_image!("../icons/square.svg"),
            Icon::SquareCheck => egui::include_image!("../icons/square-check.svg"),
            Icon::SquareChevronDown => egui::include_image!("../icons/square-chevron-down.svg"),
            Icon::SquareDashed => egui::include_image!("../icons/square-dashed.svg"),
            Icon::Terminal => egui::include_image!("../icons/terminal.svg"),
            Icon::TextCursorInput => egui::include_image!("../icons/text-cursor-input.svg"),
            Icon::Trash2 => egui::include_image!("../icons/trash-2.svg"),
            Icon::TriangleAlert => egui::include_image!("../icons/triangle-alert.svg"),
            Icon::Type => egui::include_image!("../icons/type.svg"),
            Icon::Undo2 => egui::include_image!("../icons/undo-2.svg"),
            Icon::Ungroup => egui::include_image!("../icons/ungroup.svg"),
            Icon::WholeWord => egui::include_image!("../icons/whole-word.svg"),
            Icon::X => egui::include_image!("../icons/x.svg"),
            Icon::LetterText => egui::include_image!("../icons/letter-text.svg"),
            Icon::LoaderCircle => egui::include_image!("../icons/loader-circle.svg"),
            Icon::MessageSquareWarning => {
                egui::include_image!("../icons/message-square-warning.svg")
            }
            Icon::ScrollText => egui::include_image!("../icons/scroll-text.svg"),
            Icon::SquareMousePointer => egui::include_image!("../icons/square-mouse-pointer.svg"),
            Icon::ToggleLeft => egui::include_image!("../icons/toggle-left.svg"),
            Icon::Moon => egui::include_image!("../icons/moon.svg"),
            Icon::Sun => egui::include_image!("../icons/sun.svg"),
            Icon::AlignStartVertical => egui::include_image!("../icons/align-start-vertical.svg"),
            Icon::AlignCenterVertical => egui::include_image!("../icons/align-center-vertical.svg"),
            Icon::AlignEndVertical => egui::include_image!("../icons/align-end-vertical.svg"),
            Icon::AlignStartHorizontal => {
                egui::include_image!("../icons/align-start-horizontal.svg")
            }
            Icon::AlignCenterHorizontal => {
                egui::include_image!("../icons/align-center-horizontal.svg")
            }
            Icon::AlignEndHorizontal => egui::include_image!("../icons/align-end-horizontal.svg"),
            Icon::AlignHorizontalDistributeCenter => {
                egui::include_image!("../icons/align-horizontal-distribute-center.svg")
            }
            Icon::AlignVerticalDistributeCenter => {
                egui::include_image!("../icons/align-vertical-distribute-center.svg")
            }
            Icon::ClipboardPaste => egui::include_image!("../icons/clipboard-paste.svg"),
            Icon::Scissors => egui::include_image!("../icons/scissors.svg"),
            Icon::Zap => egui::include_image!("../icons/zap.svg"),
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
