//! PeroxIDE's icons: [Lucide](https://lucide.dev) 0.460.0 shapes (ISC license) in a flat,
//! colour-filled style with outlines, embedded in the binary. Each icon comes in a light
//! variant (navy outlines) and a dark one (light outlines), picked from the theme.
//!
//! The app must install egui's image loaders once (`egui_extras::install_image_loaders`).
//! To add an icon: put the Lucide SVG in `lucide/`, give it a colour in `style.py`, run
//! `python3 style.py`, and add a line to the `icons!` list below.

use eframe::egui::{self, Image, ImageSource, Ui, Vec2};

/// Icon size in points for menus, toolbars and lists.
pub const SIZE: f32 = 15.0;

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant,)*
        }

        impl Icon {
            pub const ALL: &[Icon] = &[$(Icon::$variant,)*];

            /// The SVG for a light (`dark` false) or dark theme.
            pub fn source(self, dark: bool) -> ImageSource<'static> {
                match (self, dark) {
                    $(
                        (Icon::$variant, false) => egui::include_image!(concat!("../icons/", $file, ".svg")),
                        (Icon::$variant, true) => egui::include_image!(concat!("../icons/dark/", $file, ".svg")),
                    )*
                }
            }
        }
    };
}

icons! {
    Table => "table",
    Timer => "timer",
    Languages => "languages",
    Menu => "menu",
    SquareMenu => "square-menu",
    Keyboard => "keyboard",
    MousePointerClick => "mouse-pointer-click",
    ChartSpline => "chart-spline",
    AlignCenterHorizontal => "align-center-horizontal",
    AlignStartHorizontal => "align-start-horizontal",
    AlignHorizontalDistributeCenter => "align-horizontal-distribute-center",
    AlignVerticalDistributeCenter => "align-vertical-distribute-center",
    MessageSquareWarning => "message-square-warning",
    AppWindow => "app-window",
    ArrowDown => "arrow-down",
    ArrowDownToLine => "arrow-down-to-line",
    ArrowRight => "arrow-right",
    ArrowUp => "arrow-up",
    CaseSensitive => "case-sensitive",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    CircleCheck => "circle-check",
    CircleDot => "circle-dot",
    CircleX => "circle-x",
    Code => "code",
    Columns3 => "columns-3",
    Command => "command",
    Copy => "copy",
    File => "file",
    FileCheck => "file-check",
    FileCode => "file-code",
    FileCog => "file-cog",
    FileDiff => "file-diff",
    FileImage => "file-image",
    FileSearch => "file-search",
    FileText => "file-text",
    FlaskConical => "flask-conical",
    Folder => "folder",
    FolderOpen => "folder-open",
    Gauge => "gauge",
    GitBranch => "git-branch",
    GitCommitHorizontal => "git-commit-horizontal",
    GitCompare => "git-compare",
    Grid3x3 => "grid-3x3",
    Group => "group",
    Hammer => "hammer",
    Hash => "hash",
    History => "history",
    Image => "image",
    Info => "info",
    LayoutDashboard => "layout-dashboard",
    Lightbulb => "lightbulb",
    List => "list",
    ListChecks => "list-checks",
    LogOut => "log-out",
    Magnet => "magnet",
    Minus => "minus",
    Paintbrush => "paintbrush",
    PanelTop => "panel-top",
    PencilRuler => "pencil-ruler",
    Play => "play",
    Plug => "plug",
    Plus => "plus",
    Puzzle => "puzzle",
    RectangleHorizontal => "rectangle-horizontal",
    Redo2 => "redo-2",
    Regex => "regex",
    Replace => "replace",
    ReplaceAll => "replace-all",
    RotateCcw => "rotate-ccw",
    Rows3 => "rows-3",
    Save => "save",
    SaveAll => "save-all",
    SearchCode => "search-code",
    SeparatorHorizontal => "separator-horizontal",
    Settings => "settings",
    Settings2 => "settings-2",
    ShieldCheck => "shield-check",
    SlidersHorizontal => "sliders-horizontal",
    Space => "space",
    Square => "square",
    SquareCheck => "square-check",
    SquareChevronDown => "square-chevron-down",
    SquareDashed => "square-dashed",
    Terminal => "terminal",
    TextCursorInput => "text-cursor-input",
    Trash2 => "trash-2",
    TriangleAlert => "triangle-alert",
    Type => "type",
    Undo2 => "undo-2",
    Ungroup => "ungroup",
    WholeWord => "whole-word",
    X => "x",
    LetterText => "letter-text",
    LoaderCircle => "loader-circle",
    ScrollText => "scroll-text",
    SquareMousePointer => "square-mouse-pointer",
    ToggleLeft => "toggle-left",
    Moon => "moon",
    Sun => "sun",
    AlignStartVertical => "align-start-vertical",
    AlignCenterVertical => "align-center-vertical",
    AlignEndVertical => "align-end-vertical",
    AlignEndHorizontal => "align-end-horizontal",
    ClipboardPaste => "clipboard-paste",
    Scissors => "scissors",
    Zap => "zap",
    ArrowDownToDot => "arrow-down-to-dot",
    ArrowUpFromDot => "arrow-up-from-dot",
    ArrowUpFromLine => "arrow-up-from-line",
    Bug => "bug",
    BugPlay => "bug-play",
    Circle => "circle",
    CircleDashed => "circle-dashed",
    CirclePlay => "circle-play",
    CircleSlash => "circle-slash",
    CloudDownload => "cloud-download",
    CloudUpload => "cloud-upload",
    Eye => "eye",
    GitMerge => "git-merge",
    Layers => "layers",
    ListTree => "list-tree",
    Pause => "pause",
    RedoDot => "redo-dot",
    RefreshCw => "refresh-cw",
    Variable => "variable",
}

impl Icon {
    /// The icon at the standard size, in the current theme's variant.
    pub fn image(self, ui: &Ui) -> Image<'static> {
        self.sized(ui, SIZE)
    }

    pub fn sized(self, ui: &Ui, size: f32) -> Image<'static> {
        Image::new(self.source(ui.visuals().dark_mode)).fit_to_exact_size(Vec2::splat(size))
    }

    /// The icon washed out, for something that's off (an unused option).
    pub fn faded(self, ui: &Ui) -> Image<'static> {
        self.image(ui).tint(egui::Color32::from_white_alpha(70))
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
    fn every_icon_has_both_variants_in_the_style() {
        for icon in Icon::ALL {
            for dark in [false, true] {
                let ImageSource::Bytes { bytes, .. } = icon.source(dark) else {
                    panic!("{icon:?} isn't embedded")
                };
                let text = std::str::from_utf8(&bytes).unwrap();

                assert!(
                    text.contains("<svg")
                        && text.contains("PeroxIDE colours")
                        && !text.contains("currentColor")
                        // Navy outlines on the light theme only.
                        && (text.contains("#26324d") || dark),
                    "{icon:?} dark={dark}"
                );
            }
        }
    }
}
