//! What the designer knows about Slint elements: palette entries, where they come from
//! (`std-widgets.slint` or built in), defaults for a new one, editable properties and
//! callbacks. Anything not listed still works: it's shown with its properties as source.

use std::borrow::Cow;

use perox_icons::Icon;

pub const STD_WIDGETS: &str = "std-widgets.slint";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropKind {
    /// `string`
    Text,
    /// `length` (written with `px`)
    Length,
    /// `int`
    Int,
    /// `float`
    Float,
    Bool,
    Color,
    /// `[string]` model, edited one per line.
    Items,
    /// An enum; values written as-is.
    Choice(&'static [&'static str]),
    /// Anything else, edited as a `.slint` expression.
    Source,
    /// Data of a known Slint type (`[[StandardListViewItem]]`), edited as an expression;
    /// the type lets it be used from code.
    Data(&'static str),
}

#[derive(Debug, Clone)]
pub struct PropSpec {
    pub name: Cow<'static, str>,
    pub kind: PropKind,
    pub help: &'static str,
}

const fn p(name: &'static str, kind: PropKind, help: &'static str) -> PropSpec {
    PropSpec {
        name: Cow::Borrowed(name),
        kind,
        help,
    }
}

/// A callback an element offers: `edited(text: string)`.
#[derive(Debug, Clone, Copy)]
pub struct Event {
    pub name: &'static str,
    /// `(name, type)`.
    pub params: &'static [(&'static str, &'static str)],
}

const fn e(name: &'static str, params: &'static [(&'static str, &'static str)]) -> Event {
    Event { name, params }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Layout,
    Container,
    Widget,
}

#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub name: &'static str,
    pub icon: Icon,
    pub group: Group,
    /// Import it from `std-widgets.slint` (else it's built in).
    pub std: bool,
    pub props: &'static [PropSpec],
    pub events: &'static [Event],
}

use PropKind::*;

const BOX: &[PropSpec] = &[
    p("spacing", Length, "Gap between children"),
    p("padding", Length, "Space inside the edges"),
    p(
        "alignment",
        Choice(&[
            "stretch",
            "start",
            "center",
            "end",
            "space-between",
            "space-around",
        ]),
        "Where spare space goes along the axis",
    ),
];

const GRID: &[PropSpec] = &[
    p("spacing", Length, "Gap between cells"),
    p("padding", Length, "Space inside the edges"),
];

const FONT: &[PropSpec] = &[
    p("text", Text, ""),
    p("font-size", Length, ""),
    p("font-weight", Int, "100–900 (400 normal, 700 bold)"),
    p("color", Color, ""),
    p(
        "horizontal-alignment",
        Choice(&["left", "center", "right"]),
        "",
    ),
    p(
        "vertical-alignment",
        Choice(&["top", "center", "bottom"]),
        "",
    ),
    p("wrap", Choice(&["no-wrap", "word-wrap", "char-wrap"]), ""),
    p("overflow", Choice(&["clip", "elide"]), ""),
];

pub const KINDS: &[Kind] = &[
    Kind {
        name: "VerticalBox",
        icon: Icon::Rows3,
        group: Group::Layout,
        std: true,
        props: BOX,
        events: &[],
    },
    Kind {
        name: "HorizontalBox",
        icon: Icon::Columns3,
        group: Group::Layout,
        std: true,
        props: BOX,
        events: &[],
    },
    Kind {
        name: "GridBox",
        icon: Icon::Grid3x3,
        group: Group::Layout,
        std: true,
        props: GRID,
        events: &[],
    },
    Kind {
        name: "Rectangle",
        icon: Icon::SquareDashed,
        group: Group::Container,
        std: false,
        props: &[
            p("background", Color, ""),
            p("border-width", Length, ""),
            p("border-color", Color, ""),
            p("border-radius", Length, ""),
        ],
        events: &[],
    },
    Kind {
        name: "GroupBox",
        icon: Icon::Group,
        group: Group::Container,
        std: true,
        props: &[p("title", Text, "Shown in the frame")],
        events: &[],
    },
    Kind {
        name: "TabWidget",
        icon: Icon::PanelTop,
        group: Group::Container,
        std: true,
        props: &[p("current-index", Int, "The page shown")],
        events: &[],
    },
    Kind {
        name: "Tab",
        icon: Icon::AppWindow,
        group: Group::Container,
        std: false,
        props: &[p("title", Text, "The tab's label")],
        events: &[],
    },
    Kind {
        name: "ScrollView",
        icon: Icon::ScrollText,
        group: Group::Container,
        std: true,
        props: &[
            p("viewport-width", Length, "Width of the scrolled content"),
            p("viewport-height", Length, "Height of the scrolled content"),
        ],
        events: &[],
    },
    Kind {
        name: "Text",
        icon: Icon::Type,
        group: Group::Widget,
        std: false,
        props: FONT,
        events: &[],
    },
    Kind {
        name: "Button",
        icon: Icon::RectangleHorizontal,
        group: Group::Widget,
        std: true,
        props: &[
            p("text", Text, ""),
            p("primary", Bool, "The default action's look"),
            p("checkable", Bool, "Stays down when clicked"),
            p("checked", Bool, ""),
        ],
        events: &[e("clicked", &[])],
    },
    Kind {
        name: "LineEdit",
        icon: Icon::TextCursorInput,
        group: Group::Widget,
        std: true,
        props: &[
            p("text", Text, ""),
            p("placeholder-text", Text, "Shown while empty"),
            p(
                "input-type",
                Choice(&["text", "password", "number", "decimal"]),
                "",
            ),
            p("read-only", Bool, ""),
            p("font-size", Length, ""),
            p(
                "horizontal-alignment",
                Choice(&["left", "center", "right"]),
                "",
            ),
        ],
        events: &[
            e("edited", &[("text", "string")]),
            e("accepted", &[("text", "string")]),
        ],
    },
    Kind {
        name: "TextEdit",
        icon: Icon::LetterText,
        group: Group::Widget,
        std: true,
        props: &[
            p("text", Text, ""),
            p("placeholder-text", Text, "Shown while empty"),
            p("read-only", Bool, ""),
            p("wrap", Choice(&["no-wrap", "word-wrap", "char-wrap"]), ""),
            p("font-size", Length, ""),
        ],
        events: &[e("edited", &[("text", "string")])],
    },
    Kind {
        name: "StandardButton",
        icon: Icon::RectangleHorizontal,
        group: Group::Widget,
        std: true,
        props: &[p(
            "kind",
            Choice(&[
                "ok", "cancel", "apply", "close", "reset", "help", "yes", "no", "abort", "retry",
                "ignore",
            ]),
            "Label, icon and role in a Dialog",
        )],
        events: &[e("clicked", &[])],
    },
    Kind {
        name: "CheckBox",
        icon: Icon::SquareCheck,
        group: Group::Widget,
        std: true,
        props: &[p("text", Text, ""), p("checked", Bool, "")],
        events: &[e("toggled", &[])],
    },
    Kind {
        name: "Switch",
        icon: Icon::ToggleLeft,
        group: Group::Widget,
        std: true,
        props: &[p("text", Text, ""), p("checked", Bool, "")],
        events: &[e("toggled", &[])],
    },
    Kind {
        name: "ComboBox",
        icon: Icon::SquareChevronDown,
        group: Group::Widget,
        std: true,
        props: &[
            p("model", Items, "One per line"),
            p("current-index", Int, ""),
        ],
        events: &[e("selected", &[("current-value", "string")])],
    },
    Kind {
        name: "SpinBox",
        icon: Icon::Hash,
        group: Group::Widget,
        std: true,
        props: &[
            p("value", Int, ""),
            p("minimum", Int, "(0)"),
            p("maximum", Int, "(100)"),
            p("step-size", Int, "(1)"),
        ],
        events: &[e("edited", &[("value", "int")])],
    },
    Kind {
        name: "Slider",
        icon: Icon::SlidersHorizontal,
        group: Group::Widget,
        std: true,
        props: &[
            p("value", Float, ""),
            p("minimum", Float, "(0)"),
            p("maximum", Float, "(100)"),
            p("step", Float, "(1)"),
            p("orientation", Choice(&["horizontal", "vertical"]), ""),
        ],
        events: &[
            e("changed", &[("value", "float")]),
            e("released", &[("value", "float")]),
        ],
    },
    Kind {
        name: "ProgressIndicator",
        icon: Icon::Gauge,
        group: Group::Widget,
        std: true,
        props: &[p("progress", Float, "0 to 1"), p("indeterminate", Bool, "")],
        events: &[],
    },
    Kind {
        name: "Spinner",
        icon: Icon::LoaderCircle,
        group: Group::Widget,
        std: true,
        props: &[p("progress", Float, "0 to 1"), p("indeterminate", Bool, "")],
        events: &[],
    },
    Kind {
        name: "StandardListView",
        icon: Icon::List,
        group: Group::Widget,
        std: true,
        props: &[p("current-item", Int, "Selected row (-1: none)")],
        events: &[e("current-item-changed", &[("current-item", "int")])],
    },
    Kind {
        name: "StandardTableView",
        icon: Icon::Table,
        group: Group::Widget,
        std: true,
        props: &[
            p("columns", Data("[TableColumn]"), "[{ title: \"Name\" }, …]"),
            p(
                "rows",
                Data("[[StandardListViewItem]]"),
                "[[{ text: \"a\" }, …], …]",
            ),
            p("current-row", Int, "Selected row (-1: none)"),
        ],
        events: &[
            e("current-row-changed", &[("current-row", "int")]),
            e("sort-ascending", &[("column", "int")]),
            e("sort-descending", &[("column", "int")]),
        ],
    },
    Kind {
        name: "Path",
        icon: Icon::ChartSpline,
        group: Group::Widget,
        std: false,
        props: &[
            p("commands", Text, "SVG path data: M 0 80 L 50 20 …"),
            p("stroke", Color, ""),
            p("stroke-width", Length, ""),
            p("fill", Color, ""),
            p(
                "viewbox-width",
                Float,
                "Width of the commands' coordinate space",
            ),
            p(
                "viewbox-height",
                Float,
                "Height of the commands' coordinate space",
            ),
            p("viewbox-x", Float, ""),
            p("viewbox-y", Float, ""),
        ],
        events: &[],
    },
    Kind {
        name: "Image",
        icon: Icon::Image,
        group: Group::Widget,
        std: false,
        props: &[
            p("source", Source, "@image-url(\"path.png\")"),
            p(
                "image-fit",
                Choice(&["fill", "contain", "cover", "preserve"]),
                "",
            ),
            p("colorize", Color, "Tint (for icons)"),
        ],
        events: &[],
    },
    Kind {
        name: "TouchArea",
        icon: Icon::SquareMousePointer,
        group: Group::Container,
        std: false,
        props: &[p(
            "mouse-cursor",
            Choice(&["default", "pointer", "text", "move"]),
            "",
        )],
        events: &[e("clicked", &[]), e("double-clicked", &[])],
    },
];

pub const WINDOW: Kind = Kind {
    name: "Window",
    icon: Icon::AppWindow,
    group: Group::Container,
    std: false,
    props: &[
        p("title", Text, "Title bar text"),
        p("preferred-width", Length, "Initial width"),
        p("preferred-height", Length, "Initial height"),
        p("min-width", Length, ""),
        p("min-height", Length, ""),
        p("background", Color, ""),
        p("icon", Source, "@image-url(\"icon.png\")"),
    ],
    events: &[],
};

pub const DIALOG: Kind = Kind {
    name: "Dialog",
    icon: Icon::MessageSquareWarning,
    group: Group::Container,
    std: false,
    props: WINDOW.props,
    events: &[],
};

/// The Slint type to expose a property as, or `None` if the inspector can't yet.
pub fn slint_type(spec: &PropSpec) -> Option<&'static str> {
    Some(match spec.kind {
        Text => "string",
        Length => "length",
        Int => "int",
        Float => "float",
        Bool => "bool",
        // Colors on elements are brushes (`color`, `background`, `border-color`…).
        Color => "brush",
        Items => "[string]",
        Data(ty) => ty,
        Choice(_) | Source => return None,
    })
}

/// Properties Slint lets you bind but not alias with `<=>` (`Path.commands`): using them
/// from code means binding them to a root property instead.
pub fn binding_only(kind: &str, prop: &str) -> bool {
    matches!((kind, prop), ("Path", "commands"))
}

/// Size properties every element accepts.
pub const COMMON: &[PropSpec] = &[
    p("width", Length, "Fixed width"),
    p("height", Length, "Fixed height"),
    p("min-width", Length, ""),
    p("max-width", Length, ""),
    p("min-height", Length, ""),
    p("max-height", Length, ""),
    p("preferred-width", Length, ""),
    p("preferred-height", Length, ""),
    p("visible", Bool, "(true)"),
    p("enabled", Bool, "(true) for widgets"),
];

/// Position of a child of a container without a layout.
pub const FREE_CHILD: &[PropSpec] = &[
    p("x", Length, "From the parent's left"),
    p("y", Length, "From the parent's top"),
];

/// Properties of a child of a layout.
pub const LAYOUT_CHILD: &[PropSpec] = &[
    p("horizontal-stretch", Float, "Share of spare width"),
    p("vertical-stretch", Float, "Share of spare height"),
];

/// Extra properties for a child of a `GridBox`.
pub const GRID_CHILD: &[PropSpec] = &[
    p("row", Int, "0-based"),
    p("col", Int, "0-based"),
    p("rowspan", Int, ""),
    p("colspan", Int, ""),
];

pub fn kind(name: &str) -> Option<&'static Kind> {
    match name {
        "Window" => Some(&WINDOW),
        "Dialog" => Some(&DIALOG),
        _ => KINDS.iter().find(|k| k.name == name),
    }
}

pub fn icon(name: &str) -> Icon {
    kind(name).map_or(Icon::Puzzle, |k| k.icon)
}

/// Layouts place their children; everything else that takes children places them by
/// `x`/`y` (free placement).
pub fn is_layout(kind: &str) -> bool {
    matches!(
        kind,
        "VerticalBox"
            | "HorizontalBox"
            | "GridBox"
            | "VerticalLayout"
            | "HorizontalLayout"
            | "GridLayout"
            | "GroupBox"
    )
}

/// `GroupBox` lays its children out in a single grid cell, so it takes one (usually a
/// layout).
pub fn takes_one_child(kind: &str) -> bool {
    matches!(kind, "GroupBox" | "ScrollView")
}

/// Kinds that accept children; `TabWidget` takes only `Tab`s, and `Tab` goes nowhere else.
pub fn accepts(parent: &str, child: &str) -> bool {
    match parent {
        "TabWidget" => child == "Tab",
        _ if child == "Tab" => false,
        "Window" | "Dialog" | "PopupWindow" | "Rectangle" | "Tab" | "TouchArea" | "GroupBox"
        | "ScrollView" | "Flickable" => true,
        _ => is_layout(parent),
    }
}

/// Is a container that places children by `x`/`y`.
pub fn is_free(kind: &str) -> bool {
    !is_layout(kind) && accepts(kind, "Rectangle")
}

/// The axis children flow along in a layout.
pub fn is_horizontal(kind: &str) -> bool {
    matches!(kind, "HorizontalBox" | "HorizontalLayout")
}

pub fn is_grid(kind: &str) -> bool {
    matches!(kind, "GridBox" | "GridLayout")
}

/// Properties a freshly dropped element gets, as expressions.
pub fn defaults(kind: &str) -> Vec<(String, String)> {
    let s = |k: &str, v: &str| (k.to_owned(), perox_slint::syntax::quote(v));
    let n = |k: &str, v: &str| (k.to_owned(), v.to_owned());
    match kind {
        "Text" => vec![s("text", "Label")],
        "Button" => vec![s("text", "Button")],
        "StandardButton" => vec![n("kind", "ok")],
        "CheckBox" => vec![s("text", "Check box")],
        "Switch" => vec![s("text", "Switch")],
        "GroupBox" => vec![s("title", "Group")],
        "Tab" => vec![s("title", "Page")],
        "ComboBox" => vec![n("model", "[\"One\", \"Two\", \"Three\"]")],
        "StandardListView" => vec![n(
            "model",
            "[{ text: \"One\" }, { text: \"Two\" }, { text: \"Three\" }]",
        )],
        "StandardTableView" => vec![
            n("columns", "[{ title: \"Name\" }, { title: \"Value\" }]"),
            n(
                "rows",
                "[[{ text: \"One\" }, { text: \"1\" }], [{ text: \"Two\" }, { text: \"2\" }]]",
            ),
        ],
        "Path" => vec![
            s("commands", "M 0 80 L 25 45 L 50 60 L 75 25 L 100 35"),
            n("viewbox-width", "100"),
            n("viewbox-height", "100"),
            n("stroke", "#3b82f6"),
            n("stroke-width", "2px"),
        ],
        "Slider" => vec![n("value", "50")],
        "ProgressIndicator" => vec![n("progress", "0.5")],
        "Rectangle" => vec![n("border-width", "1px"), n("border-color", "#80808080")],
        _ => Vec::new(),
    }
}

/// Starting size for an element dropped into a free area, when it has no natural size.
pub fn free_default_size(kind: &str) -> Option<(f64, f64)> {
    match kind {
        "Rectangle" | "GroupBox" | "VerticalBox" | "HorizontalBox" | "GridBox" | "TouchArea"
        | "ScrollView" => Some((200.0, 120.0)),
        "TabWidget" => Some((280.0, 180.0)),
        "StandardListView" | "TextEdit" => Some((160.0, 120.0)),
        "StandardTableView" => Some((260.0, 140.0)),
        "Path" => Some((160.0, 100.0)),
        _ => None,
    }
}

/// `LineEdit` → `line-edit`, `TabWidget` → `tab-widget`.
pub fn kebab(kind: &str) -> String {
    let mut out = String::new();
    for (i, c) in kind.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `line-edit` → `line_edit`: Rust-friendly names for ids and handlers.
pub fn snake(kind: &str) -> String {
    kebab(kind).replace('-', "_")
}

/// Elements that usually need an id (code reads or handles them). Layouts, containers
/// and labels stay anonymous until you name them.
pub fn wants_id(kind: &str) -> bool {
    !matches!(
        kind,
        "VerticalBox"
            | "HorizontalBox"
            | "GridBox"
            | "Rectangle"
            | "GroupBox"
            | "Tab"
            | "Text"
            | "ScrollView"
            | "Spinner"
            | "StandardButton"
    )
}

/// The events of a kind, falling back to nothing for unknown kinds.
pub fn events(kind: &str) -> &'static [Event] {
    self::kind(kind).map_or(&[], |k| k.events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_rules() {
        assert!(accepts("VerticalBox", "Button"));
        assert!(accepts("TabWidget", "Tab"));
        assert!(!accepts("TabWidget", "Button"));
        assert!(!accepts("VerticalBox", "Tab"));
        assert!(!accepts("Button", "Text"));
        assert!(is_free("Window") && is_free("Rectangle") && is_free("Tab"));
        assert!(!is_free("VerticalBox") && !is_free("Button"));
        assert_eq!(kebab("StandardListView"), "standard-list-view");
        assert_eq!(snake("LineEdit"), "line_edit");
    }
}
