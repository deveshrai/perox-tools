//! Editor colours for dark and light mode.

use eframe::egui::{Color32, Visuals};

use crate::completion::Kind;
use crate::syntax::Hl;
use crate::view::Severity;

#[derive(Debug, Clone)]
pub struct Theme {
    pub text: Color32,
    pub line_number: Color32,
    pub gutter_bg: Color32,
    pub current_line: Color32,
    pub selection: Color32,
    pub cursor: Color32,
    pub bracket: Color32,
    pub find_match: Color32,
    pub find_current: Color32,
    pub inlay: Color32,
    pub inlay_bg: Color32,
    pub vcs_added: Color32,
    pub vcs_modified: Color32,
    pub vcs_deleted: Color32,
    dark: bool,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    pub fn for_visuals(v: &Visuals) -> Self {
        if v.dark_mode {
            Self {
                text: rgb(0xd4d4d4),
                line_number: rgb(0x6e7681),
                gutter_bg: v.panel_fill,
                current_line: Color32::from_white_alpha(6),
                selection: Color32::from_rgba_unmultiplied(38, 79, 120, 200),
                cursor: rgb(0xaeafad),
                bracket: rgb(0x888888),
                find_match: Color32::from_rgba_unmultiplied(234, 92, 0, 70),
                find_current: Color32::from_rgba_unmultiplied(81, 92, 106, 220),
                inlay: rgb(0x8b949e),
                inlay_bg: Color32::from_white_alpha(10),
                vcs_added: rgb(0x2ea043),
                vcs_modified: rgb(0x0078d4),
                vcs_deleted: rgb(0xf85149),
                dark: true,
            }
        } else {
            Self {
                text: rgb(0x1f2328),
                line_number: rgb(0x8c959f),
                gutter_bg: v.panel_fill,
                current_line: Color32::from_black_alpha(8),
                selection: Color32::from_rgba_unmultiplied(173, 214, 255, 200),
                cursor: rgb(0x1f2328),
                bracket: rgb(0x999999),
                find_match: Color32::from_rgba_unmultiplied(234, 92, 0, 60),
                find_current: Color32::from_rgba_unmultiplied(168, 172, 148, 200),
                inlay: rgb(0x6e7781),
                inlay_bg: Color32::from_black_alpha(10),
                vcs_added: rgb(0x1a7f37),
                vcs_modified: rgb(0x0969da),
                vcs_deleted: rgb(0xcf222e),
                dark: false,
            }
        }
    }

    pub fn color(&self, hl: Hl) -> Color32 {
        let (dark, light) = match hl {
            Hl::Keyword | Hl::SelfKw => (0x569cd6, 0x0000ff),
            Hl::Unsafe => (0xff6b6b, 0xc00000),
            Hl::Function | Hl::Method => (0xdcdcaa, 0x795e26),
            Hl::Macro => (0x4fc1ff, 0x0070c1),
            Hl::Type | Hl::BuiltinType => (0x4ec9b0, 0x267f99),
            Hl::Constant => (0x4fc1ff, 0x0070c1),
            Hl::Number => (0xb5cea8, 0x098658),
            Hl::String => (0xce9178, 0xa31515),
            Hl::Escape => (0xd7ba7d, 0xee0000),
            Hl::Comment => (0x6a9955, 0x008000),
            Hl::DocComment => (0x7fa86f, 0x2c7a2c),
            Hl::Attribute => (0x9cdcfe, 0x795e26),
            Hl::Property => (0x9cdcfe, 0x001080),
            Hl::Label | Hl::Lifetime => (0xc586c0, 0xaf00db),
            Hl::Operator | Hl::Punctuation => (0xb4b4b4, 0x444444),
            Hl::Variable => (0x9cdcfe, 0x001080),
            Hl::Parameter => (0x9cdcfe, 0x001080),
            Hl::Mutable => (0xffd580, 0x9a6700),
            Hl::Namespace => (0x4ec9b0, 0x267f99),
        };
        rgb(if self.dark { dark } else { light })
    }

    pub fn severity(&self, s: Severity) -> Color32 {
        match s {
            Severity::Error => rgb(if self.dark { 0xf14c4c } else { 0xe51400 }),
            Severity::Warning => rgb(if self.dark { 0xcca700 } else { 0xbf8803 }),
            Severity::Info => rgb(0x3794ff),
            Severity::Hint => rgb(0x888888),
        }
    }

    pub fn kind_color(&self, k: Kind) -> Color32 {
        match k {
            Kind::Function | Kind::Method => self.color(Hl::Function),
            Kind::Struct | Kind::Enum | Kind::Trait | Kind::TypeParam => self.color(Hl::Type),
            Kind::Field | Kind::Variable => self.color(Hl::Variable),
            Kind::Constant | Kind::EnumMember => self.color(Hl::Constant),
            Kind::Keyword => self.color(Hl::Keyword),
            Kind::Macro => self.color(Hl::Macro),
            Kind::Module => self.color(Hl::Namespace),
            Kind::Snippet | Kind::Other => self.line_number,
        }
    }
}
