//! Turns an element tree into a taffy tree, measures leaves with egui fonts, and
//! computes every element's rect.

use std::collections::HashMap;

use crate::egui;
use egui::{FontId, Rect, TextStyle, Vec2, vec2};
use perox_forms::{Element, Value};
use taffy::prelude::*;

pub(crate) type Values = HashMap<String, HashMap<String, Value>>;

/// A property's current value: runtime override first, then the file.
pub(crate) fn prop<'a>(
    el: &'a Element,
    values: &'a Values,
    key: &str,
    name: &str,
) -> Option<&'a Value> {
    values
        .get(key)
        .and_then(|m| m.get(name))
        .or_else(|| el.prop(name))
}

pub(crate) fn num(el: &Element, values: &Values, key: &str, name: &str) -> Option<f32> {
    prop(el, values, key, name)
        .and_then(Value::as_f64)
        .map(|n| n as f32)
}

pub(crate) fn flag(el: &Element, values: &Values, key: &str, name: &str) -> Option<bool> {
    prop(el, values, key, name).and_then(Value::as_bool)
}

pub(crate) fn text(el: &Element, values: &Values, key: &str, name: &str) -> String {
    match prop(el, values, key, name) {
        Some(Value::Str(s) | Value::Ident(s)) => s.clone(),
        Some(Value::Num(n)) => perox_forms::Value::Num(*n).to_source(),
        _ => String::new(),
    }
}

pub(crate) fn items(el: &Element, values: &Values, key: &str) -> Vec<String> {
    match prop(el, values, key, "items") {
        Some(Value::List(items)) => items
            .iter()
            .map(|v| match v {
                Value::Str(s) | Value::Ident(s) => s.clone(),
                other => other.to_source(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The current tab of a `Tabs`, or selected row of a list.
pub(crate) fn selected(el: &Element, values: &Values, key: &str) -> Option<usize> {
    num(el, values, key, "selected")
        .filter(|n| *n >= 0.0)
        .map(|n| n as usize)
}

/// Kinds that lay out children.
pub(crate) fn is_container(kind: &str) -> bool {
    matches!(
        kind,
        "Window" | "VBox" | "HBox" | "Grid" | "GroupBox" | "Tabs" | "Tab" | "Panel"
    )
}

/// A container without a layout: children are placed at `x`, `y` (like Qt widgets
/// without a layout, or a VB form). `Panel` always; `Window`, `GroupBox` and `Tab`
/// with `layout: free`.
pub fn is_free(el: &Element) -> bool {
    el.kind == "Panel"
        || (matches!(el.kind.as_str(), "Window" | "GroupBox" | "Tab")
            && el.str_prop("layout") == Some("free"))
}

/// The font a label uses, from `font_size`, `monospace` and `heading`.
pub(crate) fn label_font(el: &Element, values: &Values, key: &str, style: &egui::Style) -> FontId {
    let base = if flag(el, values, key, "heading") == Some(true) {
        TextStyle::Heading
    } else if flag(el, values, key, "monospace") == Some(true) {
        TextStyle::Monospace
    } else {
        TextStyle::Body
    };
    let mut font = base.resolve(style);
    if let Some(size) = num(el, values, key, "font_size") {
        font.size = size;
    }
    font
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Part {
    Element,
    /// The row of tab titles at the top of a `Tabs`.
    TabHeader,
}

#[derive(Debug)]
pub(crate) struct Entry {
    pub path: Vec<usize>,
    pub key: String,
    pub node: NodeId,
    pub parent: Option<usize>,
    pub part: Part,
    /// Relative to the form origin; filled after layout.
    pub rect: Rect,
    /// `rect` inside border and padding: where children (and `x: 0`) start.
    pub content: Rect,
}

#[derive(Debug)]
pub(crate) struct Leaf {
    natural: Vec2,
    /// For wrapping labels: re-measure at the width taffy offers.
    wrap: Option<(String, FontId)>,
}

#[derive(Debug)]
pub(crate) struct Built {
    pub entries: Vec<Entry>,
    root_size: Vec2,
}

impl Built {
    pub fn root_size(&self) -> Vec2 {
        self.root_size
    }

    pub fn content_size(&self) -> Vec2 {
        self.entries
            .iter()
            .fold(self.root_size, |acc, e| acc.max(e.rect.max.to_vec2()))
    }

    pub fn element_rects(&self, origin: egui::Pos2) -> Vec<(Vec<usize>, Rect)> {
        self.entries
            .iter()
            .filter(|e| e.part == Part::Element)
            .map(|e| (e.path.clone(), e.rect.translate(origin.to_vec2())))
            .collect()
    }

    /// Content boxes, in the same order as [`Built::element_rects`].
    pub fn content_rects(&self, origin: egui::Pos2) -> Vec<Rect> {
        self.entries
            .iter()
            .filter(|e| e.part == Part::Element)
            .map(|e| e.content.translate(origin.to_vec2()))
            .collect()
    }
}

struct Builder<'a> {
    tree: TaffyTree<Leaf>,
    entries: Vec<Entry>,
    values: &'a Values,
    /// Designer: empty layouts get a minimum size so you can drop into them.
    design: bool,
    style: &'a egui::Style,
    painter: &'a egui::Painter,
}

/// Lay out `root` in `available` points. The root fills it; content may be larger.
pub(crate) fn build(
    ui: &egui::Ui,
    root: &Element,
    values: &Values,
    available: Vec2,
    design: bool,
) -> Built {
    let mut b = Builder {
        tree: TaffyTree::new(),
        entries: Vec::new(),
        values,
        design,
        style: ui.style(),
        painter: ui.painter(),
    };
    let mut path = Vec::new();
    let Some(root_node) = b.element(root, &mut path, None, None) else {
        return Built {
            entries: Vec::new(),
            root_size: Vec2::ZERO,
        };
    };
    // The root fills the viewport (so spacers and stretch work) but may grow.
    if let Ok(mut style) = b.tree.style(root_node).cloned() {
        style.min_size = Size {
            width: length(available.x.max(0.0)),
            height: length(available.y.max(0.0)),
        };
        let _ = b.tree.set_style(root_node, style);
    }
    let painter = b.painter;
    let space = Size {
        width: AvailableSpace::Definite(available.x.max(0.0)),
        height: AvailableSpace::Definite(available.y.max(0.0)),
    };
    let _ = b
        .tree
        .compute_layout_with_measure(root_node, space, |input, _node, leaf, _style| {
            let Some(leaf) = leaf else {
                return taffy::LayoutOutput::from_outer_size(Size::ZERO);
            };
            let mut size = leaf.natural;
            if let Some((text, font)) = &leaf.wrap {
                let width = input
                    .known_dimensions
                    .width
                    .or(match input.available_space.width {
                        AvailableSpace::Definite(w) => Some(w),
                        _ => None,
                    });
                if let Some(w) = width {
                    let g = painter.layout(
                        text.clone(),
                        font.clone(),
                        egui::Color32::WHITE,
                        w.max(1.0),
                    );
                    size = g.size();
                }
            }
            taffy::LayoutOutput::from_outer_size(Size {
                width: input.known_dimensions.width.unwrap_or(size.x),
                height: input.known_dimensions.height.unwrap_or(size.y),
            })
        });

    // Absolute rects, parents first.
    let mut rects: Vec<Rect> = Vec::with_capacity(b.entries.len());
    let mut contents: Vec<Rect> = Vec::with_capacity(b.entries.len());
    for e in &b.entries {
        let l = b.tree.layout(e.node).copied().unwrap_or_default();
        let parent = e.parent.map_or(egui::Pos2::ZERO, |p| rects[p].min);
        let min = parent + vec2(l.location.x, l.location.y);
        let rect = Rect::from_min_size(min, vec2(l.size.width, l.size.height));
        rects.push(rect);
        contents.push(Rect::from_min_max(
            rect.min + vec2(l.border.left + l.padding.left, l.border.top + l.padding.top),
            rect.max
                - vec2(
                    l.border.right + l.padding.right,
                    l.border.bottom + l.padding.bottom,
                ),
        ));
    }
    for ((e, r), c) in b.entries.iter_mut().zip(rects).zip(contents) {
        e.rect = r;
        e.content = c;
    }
    let root_size = b.entries.first().map_or(Vec2::ZERO, |e| e.rect.size());
    Built {
        entries: b.entries,
        root_size,
    }
}

/// The parent's layout direction, for defaults that depend on it.
#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Column,
    Row,
    Grid,
    /// No layout: children sit at `x`, `y`. Carries the container's padding, which
    /// absolute positions are offset by.
    Free(f32, f32),
}

impl Builder<'_> {
    fn element(
        &mut self,
        el: &Element,
        path: &mut Vec<usize>,
        parent: Option<usize>,
        parent_flow: Option<(Flow, bool)>,
    ) -> Option<NodeId> {
        let key = crate::key(el, path);
        if flag(el, self.values, &key, "visible") == Some(false) {
            return None;
        }
        let mut style = self.common_style(el, &key, parent_flow);
        // Qt-like size policies: a window's (or tab's, or group's) layout fills it, and
        // tab sets, lists and multi-line text boxes take spare height.
        if el.prop("stretch").is_none()
            && el.prop("height").is_none()
            && let Some((Flow::Column, fills)) = parent_flow
        {
            let multi_line = el.kind == "TextEdit"
                && (flag(el, self.values, &key, "multiline") == Some(true)
                    || num(el, self.values, &key, "rows").is_some_and(|r| r > 1.0));
            let layout = matches!(
                el.kind.as_str(),
                "VBox" | "HBox" | "Grid" | "Tabs" | "Panel"
            );
            if (fills && layout)
                || matches!(el.kind.as_str(), "Tabs" | "Tab" | "ListView")
                || multi_line
            {
                style.flex_grow = 1.0;
            }
        }
        let index = self.entries.len();
        // Placeholder node; replaced once children exist.
        let node = if is_container(&el.kind) {
            let flow = self.container_style(el, &key, &mut style);
            // A lone layout inside a window, tab or group box fills it.
            let fills =
                matches!(el.kind.as_str(), "Window" | "Tab" | "GroupBox") && el.children.len() == 1;
            let placeholder = self.tree.new_leaf(Style::default()).ok()?;
            self.entries.push(Entry {
                path: path.clone(),
                key: key.clone(),
                node: placeholder,
                parent,
                part: Part::Element,
                rect: Rect::NOTHING,
                content: Rect::NOTHING,
            });
            let mut children = Vec::new();
            if el.kind == "Tabs" {
                children.extend(self.tab_header(el, &key, path, index));
                let current = selected(el, self.values, &key).unwrap_or(0);
                let visible: Vec<usize> = (0..el.children.len()).collect();
                let current = visible.get(current).copied().or(visible.first().copied());
                if let Some(i) = current {
                    path.push(i);
                    children.extend(self.element(
                        &el.children[i],
                        path,
                        Some(index),
                        Some((Flow::Column, true)),
                    ));
                    path.pop();
                }
            } else {
                for (i, child) in el.children.iter().enumerate() {
                    path.push(i);
                    children.extend(self.element(child, path, Some(index), Some((flow, fills))));
                    path.pop();
                }
            }
            if let Flow::Free(px, py) = flow {
                // Absolute children don't size their parent; make room for them.
                let (w, h) = self.free_extent(el, path, &children);
                let grow = |d: LengthPercentageAuto, n: f32| {
                    if d == LengthPercentageAuto::auto() {
                        length(n)
                    } else {
                        d
                    }
                };
                style.min_size = Size {
                    width: grow(style.min_size.width, w + 2.0 * px),
                    height: grow(style.min_size.height, h + py + px),
                };
            }
            if self.design && children.is_empty() {
                let min = |d: LengthPercentageAuto, n: f32| {
                    if d == LengthPercentageAuto::auto() {
                        length(n)
                    } else {
                        d
                    }
                };
                style.min_size = Size {
                    width: min(style.min_size.width, 48.0),
                    height: min(style.min_size.height, 28.0),
                };
            }
            let node = self.tree.new_with_children(style, &children).ok()?;
            self.entries[index].node = node;
            node
        } else {
            let leaf = self.measure(el, &key, parent_flow.map(|f| f.0), &mut style);
            let node = self.tree.new_leaf_with_context(style, leaf).ok()?;
            self.entries.push(Entry {
                path: path.clone(),
                key,
                node,
                parent,
                part: Part::Element,
                rect: Rect::NOTHING,
                content: Rect::NOTHING,
            });
            node
        };
        Some(node)
    }

    fn tab_header(
        &mut self,
        el: &Element,
        key: &str,
        path: &[usize],
        parent: usize,
    ) -> Option<NodeId> {
        let font = TextStyle::Button.resolve(self.style);
        let sp = &self.style.spacing;
        let width: f32 = el
            .children
            .iter()
            .map(|t| {
                let title = t.str_prop("title").unwrap_or(&t.kind).to_owned();
                self.text_size(&title, &font).x + 2.0 * sp.button_padding.x + sp.item_spacing.x
            })
            .sum();
        let _ = key;
        let leaf = Leaf {
            natural: vec2(width, sp.interact_size.y + 4.0),
            wrap: None,
        };
        let node = self
            .tree
            .new_leaf_with_context(Style::default(), leaf)
            .ok()?;
        self.entries.push(Entry {
            path: path.to_vec(),
            key: crate::key(el, path),
            node,
            parent: Some(parent),
            part: Part::TabHeader,
            rect: Rect::NOTHING,
            content: Rect::NOTHING,
        });
        Some(node)
    }

    /// Size and grid placement properties every element accepts.
    fn common_style(&self, el: &Element, key: &str, parent_flow: Option<(Flow, bool)>) -> Style {
        let v = self.values;
        let mut s = Style::default();
        let dim = |name: &str| num(el, v, key, name).map_or(Dimension::auto(), length);
        let lim = |name: &str| num(el, v, key, name).map_or(LengthPercentageAuto::auto(), length);
        s.size = Size {
            width: dim("width"),
            height: dim("height"),
        };
        if let Some(Value::Size(w, h)) = prop(el, v, key, "fixed_size") {
            s.size = Size::from_lengths(*w as f32, *h as f32);
        }
        s.min_size = Size {
            width: lim("min_width"),
            height: lim("min_height"),
        };
        s.max_size = Size {
            width: lim("max_width"),
            height: lim("max_height"),
        };
        if let Some(n) = num(el, v, key, "stretch") {
            s.flex_grow = n;
            s.flex_basis = length(0.0);
        }
        s.flex_shrink = 1.0;
        if let Some((Flow::Free(px, py), _)) = parent_flow {
            let at = |name: &str| num(el, v, key, name).unwrap_or(0.0);
            s.position = Position::Absolute;
            s.inset = taffy::Rect {
                left: length(px + at("x")),
                top: length(py + at("y")),
                right: LengthPercentageAuto::auto(),
                bottom: LengthPercentageAuto::auto(),
            };
        }
        if let Some((Flow::Grid, _)) = parent_flow {
            let at = |name: &str| num(el, v, key, name).map(|n| n.max(0.0) as i16);
            let span_of = |name: &str| num(el, v, key, name).map_or(1, |n| n.max(1.0) as u16);
            if let Some(r) = at("row") {
                s.grid_row = Line {
                    start: line(r + 1),
                    end: span(span_of("row_span")),
                };
            }
            if let Some(c) = at("column") {
                s.grid_column = Line {
                    start: line(c + 1),
                    end: span(span_of("column_span")),
                };
            }
        }
        s
    }

    /// Flex or grid settings for a container. Returns how it flows its children.
    fn container_style(&self, el: &Element, key: &str, s: &mut Style) -> Flow {
        let v = self.values;
        let sp = &self.style.spacing;
        let spacing = num(el, v, key, "spacing").unwrap_or(6.0);
        let free = is_free(el);
        let default_margin = match el.kind.as_str() {
            _ if free && el.kind != "GroupBox" => 0.0,
            "Window" => 10.0,
            "GroupBox" | "Tab" => 8.0,
            _ => 0.0,
        };
        let margin = num(el, v, key, "margin").unwrap_or(default_margin);
        s.padding = taffy::Rect {
            left: length(margin),
            right: length(margin),
            top: length(margin),
            bottom: length(margin),
        };
        if el.kind == "GroupBox" {
            let font = TextStyle::Body.resolve(self.style);
            let title_h = self.text_size("Ag", &font).y;
            s.padding.top = length(margin + title_h + 2.0);
            s.border = taffy::Rect {
                left: length(1.0),
                right: length(1.0),
                top: length(1.0),
                bottom: length(1.0),
            };
        }
        s.gap = Size {
            width: length(spacing),
            height: length(spacing),
        };
        let _ = sp;
        let title = if el.kind == "GroupBox" {
            self.text_size("Ag", &TextStyle::Body.resolve(self.style)).y + 2.0
        } else {
            0.0
        };
        let pad_top = margin + title;
        let flow = if free {
            Flow::Free(margin, pad_top)
        } else if el.kind == "HBox" {
            Flow::Row
        } else if el.kind == "Grid" {
            Flow::Grid
        } else {
            Flow::Column
        };
        match flow {
            Flow::Grid => {
                s.display = Display::Grid;
                s.grid_template_columns = grid_columns(el.prop("columns"));
                s.align_items = AlignItems::CENTER;
                // Spare height goes below the rows, not between them.
                s.align_content = AlignContent::START;
            }
            Flow::Row | Flow::Column | Flow::Free(..) => {
                s.display = Display::Flex;
                s.flex_direction = if flow == Flow::Row {
                    FlexDirection::Row
                } else {
                    FlexDirection::Column
                };
            }
        }
        if let Some(a) = el.str_prop("align") {
            s.align_items = match a {
                "start" | "top" | "left" => AlignItems::START,
                "end" | "bottom" | "right" => AlignItems::END,
                "center" => AlignItems::CENTER,
                _ => AlignItems::STRETCH,
            };
        }
        if let Some(j) = el.str_prop("justify") {
            s.justify_content = match j {
                "end" | "bottom" | "right" => JustifyContent::END,
                "center" => JustifyContent::CENTER,
                "between" | "space_between" => JustifyContent::SPACE_BETWEEN,
                "around" | "space_around" => JustifyContent::SPACE_AROUND,
                _ => JustifyContent::START,
            };
        }
        flow
    }

    fn text_size(&self, text: &str, font: &FontId) -> Vec2 {
        self.painter
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
            .size()
    }

    /// The bottom-right corner of a free container's children: `x + width`, `y + height`,
    /// using natural sizes where no size is set.
    fn free_extent(&self, el: &Element, path: &[usize], children: &[NodeId]) -> (f32, f32) {
        let v = self.values;
        let mut nodes = children.iter();
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for (i, child) in el.children.iter().enumerate() {
            let mut p = path.to_vec();
            p.push(i);
            let key = crate::key(child, &p);
            if flag(child, v, &key, "visible") == Some(false) {
                continue;
            }
            let natural = nodes
                .next()
                .and_then(|n| self.tree.get_node_context(*n))
                .map_or(Vec2::ZERO, |leaf| leaf.natural);
            let cw = num(child, v, &key, "width").unwrap_or(natural.x);
            let ch = num(child, v, &key, "height").unwrap_or(natural.y);
            w = w.max(num(child, v, &key, "x").unwrap_or(0.0) + cw);
            h = h.max(num(child, v, &key, "y").unwrap_or(0.0) + ch);
        }
        (w, h)
    }

    /// A leaf's natural size, plus per-kind style tweaks.
    fn measure(&self, el: &Element, key: &str, parent: Option<Flow>, s: &mut Style) -> Leaf {
        let v = self.values;
        let sp = &self.style.spacing;
        let h = sp.interact_size.y;
        let body = TextStyle::Body.resolve(self.style);
        let button = TextStyle::Button.resolve(self.style);
        let row = self.text_size("Ag", &body).y;
        let mut wrap = None;
        // Fixed-height widgets sit centred in rows and grids, like Qt's size policies.
        let centre = matches!(parent, Some(Flow::Row | Flow::Grid));
        let natural = match el.kind.as_str() {
            "Label" => {
                let font = label_font(el, v, key, self.style);
                let t = text(el, v, key, "text");
                if flag(el, v, key, "wrap") == Some(true) {
                    wrap = Some((t.clone(), font.clone()));
                    s.flex_shrink = 1.0;
                    s.min_size.width = length(20.0);
                }
                // The designer shows empty labels by name, so you can find them.
                let shown = if t.is_empty() && self.design {
                    placeholder_text(el)
                } else {
                    t
                };
                self.text_size(&shown, &font)
            }
            "Button" => {
                let t = self.text_size(&text(el, v, key, "text"), &button);
                vec2(
                    (t.x + 2.0 * sp.button_padding.x).max(sp.interact_size.x),
                    (t.y + 2.0 * sp.button_padding.y).max(h),
                )
            }
            "CheckBox" | "RadioButton" => {
                let t = self.text_size(&text(el, v, key, "text"), &body);
                vec2(sp.icon_width + sp.icon_spacing + t.x, t.y.max(h))
            }
            "TextEdit" => {
                let rows = num(el, v, key, "rows").unwrap_or(1.0).max(1.0);
                let multi = flag(el, v, key, "multiline") == Some(true) || rows > 1.0;
                let rows = if multi && el.prop("rows").is_none() {
                    4.0
                } else {
                    rows
                };
                vec2(160.0, (rows * row + 8.0).max(h))
            }
            "ComboBox" => {
                let widest = items(el, v, key)
                    .iter()
                    .map(|i| self.text_size(i, &button).x)
                    .fold(0.0, f32::max);
                vec2(
                    (widest + sp.icon_width + sp.icon_spacing + 2.0 * sp.button_padding.x + 8.0)
                        .max(100.0),
                    h,
                )
            }
            "Slider" => vec2(sp.slider_width + 60.0, h),
            "SpinBox" => vec2(70.0, h),
            "ProgressBar" => vec2(160.0, h),
            "Image" => match prop(el, v, key, "size") {
                Some(Value::Size(w, h)) => vec2(*w as f32, *h as f32),
                _ => vec2(64.0, 64.0),
            },
            "Separator" => {
                s.align_self = Some(AlignSelf::STRETCH);
                if parent == Some(Flow::Row) {
                    vec2(6.0, 0.0)
                } else {
                    vec2(0.0, 6.0)
                }
            }
            "ListView" => {
                let rows = num(el, v, key, "rows").unwrap_or(5.0).max(1.0);
                vec2(160.0, rows * (row + sp.item_spacing.y) + 4.0)
            }
            "Spacer" => {
                if el.prop("stretch").is_none() {
                    s.flex_grow = 1.0;
                }
                let n = num(el, v, key, "size").unwrap_or(0.0);
                vec2(n, n)
            }
            other => {
                // Unknown widget (third-party component without a runtime): a labelled box.
                let t = self.text_size(other, &body);
                vec2(t.x + 16.0, t.y + 12.0)
            }
        };
        if centre && el.kind != "Separator" && el.kind != "Spacer" && s.align_self.is_none() {
            s.align_self = Some(AlignSelf::CENTER);
        }
        // Text boxes, sliders and lists grow along a row by default.
        if parent == Some(Flow::Row)
            && matches!(
                el.kind.as_str(),
                "TextEdit" | "Slider" | "ProgressBar" | "ListView"
            )
            && el.prop("stretch").is_none()
        {
            s.flex_grow = 1.0;
        }
        Leaf { natural, wrap }
    }
}

/// `columns: 3` → three equal columns; `columns: [0, 1]` → a fitted column and a
/// stretching one (0 means "fit content", n means n shares of the rest).
fn grid_columns(spec: Option<&Value>) -> Vec<GridTemplateComponent<String>> {
    let track =
        |w: f64| -> GridTemplateComponent<String> { if w <= 0.0 { auto() } else { fr(w as f32) } };
    match spec {
        Some(Value::Num(n)) => (0..(*n as usize).max(1)).map(|_| track(1.0)).collect(),
        Some(Value::List(list)) => list
            .iter()
            .map(|v| track(v.as_f64().unwrap_or(0.0)))
            .collect(),
        _ => vec![track(0.0), track(1.0)],
    }
}

/// What the designer shows for an empty label: its id, or "Label".
pub(crate) fn placeholder_text(el: &Element) -> String {
    el.id.clone().unwrap_or_else(|| el.kind.clone())
}
