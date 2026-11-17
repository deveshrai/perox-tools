//! What the designer knows about each element kind: palette entry, default
//! properties for a new one, editable properties and events.

use std::borrow::Cow;

use perox_forms::Value;
use perox_icons::Icon;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PropKind {
    Text,
    /// A whole number or decimal.
    Number,
    Bool,
    Color,
    Size,
    /// `items: [...]`, edited one per line.
    Items,
    Choice(&'static [&'static str]),
    /// Anything else, edited as `.pxui` source (`columns: [0, 1]`).
    Source,
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

#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub name: &'static str,
    pub icon: Icon,
    pub layout: bool,
    pub props: &'static [PropSpec],
    pub events: &'static [&'static str],
}

use PropKind::*;

const ALIGN: PropKind = Choice(&["stretch", "start", "center", "end"]);
const JUSTIFY: PropKind = Choice(&["start", "center", "end", "between", "around"]);
const BOX: &[PropSpec] = &[
    p("spacing", Number, "Gap between children (6)"),
    p("margin", Number, "Space inside the edges"),
    p("align", ALIGN, "Cross-axis alignment of children"),
    p("justify", JUSTIFY, "Where spare space goes along the axis"),
];

pub const KINDS: &[Kind] = &[
    Kind {
        name: "VBox",
        icon: Icon::Rows3,
        layout: true,
        props: BOX,
        events: &[],
    },
    Kind {
        name: "HBox",
        icon: Icon::Columns3,
        layout: true,
        props: BOX,
        events: &[],
    },
    Kind {
        name: "Grid",
        icon: Icon::Grid3x3,
        layout: true,
        props: &[
            p(
                "columns",
                Source,
                "[0, 1]: 0 fits content, n takes n shares",
            ),
            p("spacing", Number, "Gap between cells (6)"),
            p("margin", Number, "Space inside the edges"),
        ],
        events: &[],
    },
    Kind {
        name: "GroupBox",
        icon: Icon::Group,
        layout: true,
        props: &[
            p("title", Text, "Shown in the frame"),
            p("spacing", Number, "Gap between children (6)"),
            p("margin", Number, "Space inside the frame (8)"),
        ],
        events: &[],
    },
    Kind {
        name: "Tabs",
        icon: Icon::PanelTop,
        layout: true,
        props: &[p("selected", Number, "The page shown first (0)")],
        events: &["changed"],
    },
    Kind {
        name: "Tab",
        icon: Icon::AppWindow,
        layout: true,
        props: &[
            p("title", Text, "The tab's label"),
            p("spacing", Number, "Gap between children (6)"),
            p("margin", Number, "Space inside the page (8)"),
        ],
        events: &[],
    },
    Kind {
        name: "Label",
        icon: Icon::Type,
        layout: false,
        props: &[
            p("text", Text, ""),
            p("heading", Bool, "Large heading font"),
            p("bold", Bool, ""),
            p("color", Color, ""),
            p("font_size", Number, "Points"),
            p("wrap", Bool, "Wrap to the available width"),
            p("monospace", Bool, ""),
            p("text_align", Choice(&["left", "center", "right"]), ""),
        ],
        events: &[],
    },
    Kind {
        name: "Button",
        icon: Icon::RectangleHorizontal,
        layout: false,
        props: &[p("text", Text, "")],
        events: &["clicked"],
    },
    Kind {
        name: "TextEdit",
        icon: Icon::TextCursorInput,
        layout: false,
        props: &[
            p("text", Text, ""),
            p("placeholder", Text, "Shown while empty"),
            p("rows", Number, "More than 1 makes it multi-line"),
            p("multiline", Bool, ""),
            p("password", Bool, "Hide the characters"),
            p("read_only", Bool, ""),
            p("monospace", Bool, ""),
        ],
        events: &["changed", "submitted"],
    },
    Kind {
        name: "CheckBox",
        icon: Icon::SquareCheck,
        layout: false,
        props: &[p("text", Text, ""), p("checked", Bool, "")],
        events: &["toggled"],
    },
    Kind {
        name: "RadioButton",
        icon: Icon::CircleDot,
        layout: false,
        props: &[
            p("text", Text, ""),
            p("checked", Bool, ""),
            p(
                "group",
                Text,
                "Exclusive with others in the group (default: same parent)",
            ),
        ],
        events: &["toggled"],
    },
    Kind {
        name: "ComboBox",
        icon: Icon::SquareChevronDown,
        layout: false,
        props: &[
            p("items", Items, "One per line"),
            p("selected", Number, "Index"),
        ],
        events: &["changed"],
    },
    Kind {
        name: "Slider",
        icon: Icon::SlidersHorizontal,
        layout: false,
        props: &[
            p("value", Number, ""),
            p("min", Number, "(0)"),
            p("max", Number, "(100)"),
            p("step", Number, ""),
            p("show_value", Bool, "Show the number box (true)"),
        ],
        events: &["changed"],
    },
    Kind {
        name: "SpinBox",
        icon: Icon::Hash,
        layout: false,
        props: &[
            p("value", Number, ""),
            p("min", Number, ""),
            p("max", Number, ""),
            p("step", Number, "(1)"),
            p("prefix", Text, ""),
            p("suffix", Text, ""),
        ],
        events: &["changed"],
    },
    Kind {
        name: "ProgressBar",
        icon: Icon::Gauge,
        layout: false,
        props: &[
            p("value", Number, ""),
            p("min", Number, "(0)"),
            p("max", Number, "(100)"),
            p("text", Text, "Instead of the percentage"),
        ],
        events: &[],
    },
    Kind {
        name: "ListView",
        icon: Icon::List,
        layout: false,
        props: &[
            p("items", Items, "One per line"),
            p("selected", Number, "Index"),
            p("rows", Number, "Visible rows (5)"),
        ],
        events: &["changed", "activated"],
    },
    Kind {
        name: "Image",
        icon: Icon::Image,
        layout: false,
        props: &[
            p("source", Text, "Path or URL"),
            p("size", Size, "(64 x 64)"),
        ],
        events: &[],
    },
    Kind {
        name: "Separator",
        icon: Icon::SeparatorHorizontal,
        layout: false,
        props: &[],
        events: &[],
    },
    Kind {
        name: "Spacer",
        icon: Icon::Space,
        layout: false,
        props: &[p("size", Number, "Minimum size")],
        events: &[],
    },
];

pub const WINDOW: Kind = Kind {
    name: "Window",
    icon: Icon::AppWindow,
    layout: true,
    props: &[
        p("title", Text, "Title bar text"),
        p("size", Size, "Initial size"),
        p("min_size", Size, ""),
        p("resizable", Bool, "(true)"),
        p("spacing", Number, "Gap between children (6)"),
        p("margin", Number, "Space inside the window (10)"),
    ],
    events: &[],
};

/// Size and placement properties every element accepts.
pub const COMMON: &[PropSpec] = &[
    p("width", Number, "Fixed width"),
    p("height", Number, "Fixed height"),
    p("min_width", Number, ""),
    p("max_width", Number, ""),
    p("min_height", Number, ""),
    p("max_height", Number, ""),
    p("stretch", Number, "Share of spare space"),
    p("visible", Bool, "(true)"),
    p("enabled", Bool, "(true)"),
    p("tooltip", Text, ""),
];

/// Extra properties for a child of a `Grid`.
pub const GRID_CHILD: &[PropSpec] = &[
    p("row", Number, "0-based"),
    p("column", Number, "0-based"),
    p("row_span", Number, ""),
    p("column_span", Number, ""),
];

pub fn kind(name: &str) -> Option<&'static Kind> {
    if name == "Window" {
        return Some(&WINDOW);
    }
    KINDS.iter().find(|k| k.name == name)
}

pub fn icon(name: &str) -> Icon {
    kind(name).map_or(Icon::Puzzle, |k| k.icon)
}

/// Kinds that accept children; `Tabs` takes only `Tab`s.
pub fn accepts(parent: &str, child: &str) -> bool {
    match parent {
        "Tabs" => child == "Tab",
        "Window" | "VBox" | "HBox" | "Grid" | "GroupBox" | "Tab" => child != "Tab",
        _ => false,
    }
}

/// Properties a freshly dropped widget gets.
pub fn defaults(kind: &str) -> Vec<(String, Value)> {
    let s = |k: &str, v: &str| (k.to_owned(), Value::Str(v.to_owned()));
    let n = |k: &str, v: f64| (k.to_owned(), Value::Num(v));
    let items = |k: &str| {
        (
            k.to_owned(),
            Value::List(vec![
                Value::Str("One".into()),
                Value::Str("Two".into()),
                Value::Str("Three".into()),
            ]),
        )
    };
    match kind {
        "Label" => vec![s("text", "Label")],
        "Button" => vec![s("text", "Button")],
        "CheckBox" => vec![s("text", "Check box")],
        "RadioButton" => vec![s("text", "Option")],
        "GroupBox" => vec![s("title", "Group")],
        "Tab" => vec![s("title", "Page")],
        "ComboBox" => vec![items("items"), n("selected", 0.0)],
        "ListView" => vec![items("items")],
        "Slider" | "ProgressBar" => vec![n("value", 50.0)],
        _ => Vec::new(),
    }
}

/// `TextEdit` → `text_edit`.
pub fn snake(kind: &str) -> String {
    let mut out = String::new();
    for (i, c) in kind.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Widgets that usually need an id (code reads or handles them). Layouts and labels
/// stay anonymous until you name them.
pub fn wants_id(kind: &str) -> bool {
    !matches!(
        kind,
        "VBox" | "HBox" | "Grid" | "GroupBox" | "Tab" | "Label" | "Separator" | "Spacer"
    )
}
