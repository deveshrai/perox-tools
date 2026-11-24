//! Free placement: converting between layouts and `x`/`y` geometry, like Qt's
//! "Break Layout" and "Lay Out Vertically / Horizontally / in a Grid". In Slint, any
//! element that isn't a layout places its children by `x`/`y`, so "free" just means
//! "not a layout".
//!
//! Each operation runs several edits on a working copy of the source and returns the
//! final text; the designer turns that into one minimal edit (one undo step).

use eframe::egui::{Pos2, Rect, vec2};
use perox_slint::edit::{self, EditError, TextEdit};

use crate::catalog;

/// Apply one step to the working copy.
pub fn step(work: &mut String, edits: Result<Vec<TextEdit>, EditError>) -> Result<(), EditError> {
    let edits = edits?;
    *work = edit::apply(work, &edits);
    Ok(())
}

/// Where elements were drawn last frame (screen points).
#[derive(Debug, Clone, Copy)]
pub struct Drawn<'a> {
    pub rects: &'a [(Vec<usize>, Rect)],
}

impl Drawn<'_> {
    pub fn rect(&self, path: &[usize]) -> Option<Rect> {
        self.rects.iter().find(|(p, _)| p == path).map(|(_, r)| *r)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Vertical,
    Horizontal,
    Grid,
}

impl Direction {
    pub fn kind(self) -> &'static str {
        match self {
            Direction::Vertical => "VerticalBox",
            Direction::Horizontal => "HorizontalBox",
            Direction::Grid => "GridBox",
        }
    }
}

/// The grid free placement snaps to, in points.
pub const GRID: f32 = 8.0;

/// Snap to the grid (when on) and keep inside the container (no negative positions).
pub fn snap(v: f32, on: bool) -> f32 {
    let v = v.max(0.0);
    if on {
        (v / GRID).round() * GRID
    } else {
        v.round()
    }
}

/// A length as `.slint` source.
pub fn px(v: f32) -> String {
    format!("{}px", v.round() as i64)
}

fn set(
    work: &mut String,
    comp: &str,
    path: &[usize],
    name: &str,
    value: &str,
) -> Result<(), EditError> {
    let src = work.clone();
    step(work, edit::set_property(&src, comp, path, name, value))
}

/// Write `x`, `y`, `width`, `height` for the element at `path`.
pub fn set_geometry(
    work: &mut String,
    comp: &str,
    path: &[usize],
    r: Rect,
) -> Result<(), EditError> {
    for (name, v) in [
        ("x", r.min.x),
        ("y", r.min.y),
        ("width", r.width()),
        ("height", r.height()),
    ] {
        set(work, comp, path, name, &px(v))?;
    }
    Ok(())
}

pub fn set_position(
    work: &mut String,
    comp: &str,
    path: &[usize],
    at: Pos2,
) -> Result<(), EditError> {
    set(work, comp, path, "x", &px(at.x))?;
    set(work, comp, path, "y", &px(at.y))
}

/// Remove properties that only mean something in the old arrangement.
pub fn strip(
    work: &mut String,
    comp: &str,
    path: &[usize],
    names: &[&str],
) -> Result<(), EditError> {
    for name in names {
        let src = work.clone();
        step(work, edit::remove_property(&src, comp, path, name))?;
    }
    Ok(())
}

/// Makes sure a `std-widgets.slint` kind is imported.
pub fn import(work: &mut String, kind: &str) {
    if catalog::kind(kind).is_some_and(|k| k.std) {
        let e = edit::ensure_import(work, kind, catalog::STD_WIDGETS);
        *work = edit::apply(work, &e);
    }
}

pub const GEOMETRY: &[&str] = &["x", "y", "width", "height"];
pub const LAYOUT_CHILD: &[&str] = &[
    "row",
    "col",
    "rowspan",
    "colspan",
    "horizontal-stretch",
    "vertical-stretch",
];
const LAYOUT_SELF: &[&str] = &[
    "spacing",
    "padding",
    "padding-left",
    "padding-right",
    "padding-top",
    "padding-bottom",
    "alignment",
];
const RECTANGLE_SELF: &[&str] = &[
    "background",
    "border-width",
    "border-color",
    "border-radius",
];

fn component<'a>(
    doc: &'a perox_slint::Document,
    name: &str,
) -> Result<&'a perox_slint::Component, EditError> {
    doc.component(name)
        .ok_or_else(|| EditError::NoSuchComponent(name.into()))
}

/// Children's rects relative to `origin`, for the children that were drawn.
fn child_rects(drawn: Drawn<'_>, parent: &[usize], n: usize, origin: Pos2) -> Vec<(usize, Rect)> {
    (0..n)
        .filter_map(|i| {
            let mut p = parent.to_vec();
            p.push(i);
            drawn.rect(&p).map(|r| (i, r.translate(-origin.to_vec2())))
        })
        .collect()
}

/// Is "Break Layout" possible on this element?
pub fn can_break(comp: &perox_slint::Component, path: &[usize]) -> bool {
    let Some(el) = comp.at(path) else {
        return false;
    };
    let plain_layout = catalog::is_layout(&el.kind) && el.kind != "GroupBox" && !path.is_empty();
    let holds_one_layout = catalog::is_free(&el.kind)
        && el.children.len() == 1
        && catalog::is_layout(&el.children[0].kind)
        && el.children[0].kind != "GroupBox";
    plain_layout || holds_one_layout
}

/// "Break Layout" on the element at `path`. A layout becomes a `Rectangle`; a window,
/// tab or rectangle holding a single layout takes over that layout's children. Every
/// element keeps its place on screen.
pub fn break_layout(
    src: &str,
    comp: &str,
    drawn: Drawn<'_>,
    path: &[usize],
) -> Result<String, EditError> {
    let doc = perox_slint::parse(src);
    let c = component(&doc, comp)?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    if !can_break(c, path) {
        return Err(EditError::NoSuchElement);
    }
    let mut work = src.to_owned();
    if catalog::is_layout(&el.kind) {
        let origin = drawn.rect(path).ok_or(EditError::NoSuchElement)?.min;
        let geometry = child_rects(drawn, path, el.children.len(), origin);
        step(&mut work, edit::set_kind(src, comp, path, "Rectangle"))?;
        strip(&mut work, comp, path, LAYOUT_SELF)?;
        for (i, r) in geometry {
            let mut p = path.to_vec();
            p.push(i);
            strip(&mut work, comp, &p, LAYOUT_CHILD)?;
            set_geometry(&mut work, comp, &p, r)?;
        }
        return Ok(work);
    }
    // Hoist the single layout's children into the container, then drop the layout.
    let origin = drawn.rect(path).ok_or(EditError::NoSuchElement)?.min;
    let mut lp = path.to_vec();
    lp.push(0);
    let inner = &el.children[0];
    let geometry = child_rects(drawn, &lp, inner.children.len(), origin);
    for j in (0..inner.children.len()).rev() {
        let mut from = lp.clone();
        from.push(j);
        let s = work.clone();
        step(&mut work, edit::move_element(&s, comp, &from, path, 1))?;
    }
    let s = work.clone();
    step(&mut work, edit::remove_element(&s, comp, &lp))?;
    for (j, r) in geometry {
        let mut p = path.to_vec();
        p.push(j);
        strip(&mut work, comp, &p, LAYOUT_CHILD)?;
        set_geometry(&mut work, comp, &p, r)?;
    }
    Ok(work)
}

/// Reading order: by `y` then `x` for grids. Returns child indexes and, for each, its
/// (row, column).
fn reading_order(rects: &[(usize, Rect)], dir: Direction) -> Vec<(usize, usize, usize)> {
    let mut items: Vec<(usize, Rect)> = rects.to_vec();
    match dir {
        Direction::Vertical => {
            items.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
            items
                .iter()
                .enumerate()
                .map(|(k, (i, _))| (*i, k, 0))
                .collect()
        }
        Direction::Horizontal => {
            items.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
            items
                .iter()
                .enumerate()
                .map(|(k, (i, _))| (*i, 0, k))
                .collect()
        }
        Direction::Grid => {
            items.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
            let mut rows: Vec<Vec<(usize, Rect)>> = Vec::new();
            for it in items {
                match rows.last_mut() {
                    Some(row) if it.1.top() < row[0].1.top() + row[0].1.height().max(8.0) / 2.0 => {
                        row.push(it)
                    }
                    _ => rows.push(vec![it]),
                }
            }
            rows.into_iter()
                .enumerate()
                .flat_map(|(r, mut row)| {
                    row.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
                    row.into_iter()
                        .enumerate()
                        .map(move |(c, (i, _))| (i, r, c))
                })
                .collect()
        }
    }
}

/// Is "Lay Out" possible on this element?
pub fn can_lay_out(comp: &perox_slint::Component, path: &[usize]) -> bool {
    comp.at(path).is_some_and(|el| {
        catalog::is_free(&el.kind) && el.kind != "TabWidget" && !el.children.is_empty()
    })
}

/// "Lay Out Vertically / Horizontally / in a Grid" on a container at `path`, ordering
/// children by where they sit. A nested `Rectangle` becomes the layout; a window, tab or
/// top-level rectangle gets a new layout holding its children.
pub fn lay_out(
    src: &str,
    comp: &str,
    drawn: Drawn<'_>,
    path: &[usize],
    dir: Direction,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(src);
    let c = component(&doc, comp)?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    if !can_lay_out(c, path) {
        return Err(EditError::NoSuchElement);
    }
    let n = el.children.len();
    let mut rects = child_rects(drawn, path, n, Pos2::ZERO);
    // Hidden children have no rect; keep them, at the end.
    for i in 0..n {
        if !rects.iter().any(|(j, _)| *j == i) {
            rects.push((
                i,
                Rect::from_min_size(Pos2::new(f32::MAX / 4.0, f32::MAX / 4.0), vec2(1.0, 1.0)),
            ));
        }
    }
    let order = reading_order(&rects, dir);
    let convert = el.kind == "Rectangle" && !path.is_empty();
    let mut work = src.to_owned();
    let layout: Vec<usize> = if convert {
        path.to_vec()
    } else {
        step(
            &mut work,
            edit::add_element(src, comp, path, 0, dir.kind(), None, &[]),
        )?;
        let mut lp = path.to_vec();
        lp.push(0);
        lp
    };
    let offset = usize::from(!convert);
    let mut current: Vec<usize> = (0..n).collect();
    for (k, (child, _, _)) in order.iter().enumerate() {
        let pos = current
            .iter()
            .position(|c| c == child)
            .expect("each child once");
        let s = work.clone();
        if convert {
            if pos != k {
                let mut from = path.to_vec();
                from.push(pos);
                step(&mut work, edit::move_element(&s, comp, &from, path, k))?;
                let moved = current.remove(pos);
                current.insert(k, moved);
            }
        } else {
            let mut from = path.to_vec();
            from.push(offset + pos);
            step(&mut work, edit::move_element(&s, comp, &from, &layout, k))?;
            current.remove(pos);
        }
    }
    for (k, (_, row, col)) in order.iter().enumerate() {
        let mut p = layout.clone();
        p.push(k);
        strip(&mut work, comp, &p, GEOMETRY)?;
        if dir == Direction::Grid {
            set(&mut work, comp, &p, "row", &row.to_string())?;
            set(&mut work, comp, &p, "col", &col.to_string())?;
        }
    }
    if convert {
        let s = work.clone();
        step(&mut work, edit::set_kind(&s, comp, path, dir.kind()))?;
        strip(&mut work, comp, path, RECTANGLE_SELF)?;
    }
    import(&mut work, dir.kind());
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn check(out: &str) -> perox_slint::Component {
        let d = perox_slint::parse(out);
        assert!(d.errors.is_empty(), "{out}\n{:?}", d.errors);
        d.components.into_iter().next().unwrap()
    }

    #[test]
    fn break_a_box_into_a_rectangle_keeping_positions() {
        let src = "export component W inherits Window {\n    VerticalBox {\n        spacing: 4px;\n        a := Button { text: \"A\"; }\n        b := Button { text: \"B\"; }\n    }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(10.0, 10.0, 280.0, 180.0)),
            (vec![0, 0], r(10.0, 10.0, 280.0, 24.0)),
            (vec![0, 1], r(10.0, 38.0, 280.0, 24.0)),
        ];
        // The window's single layout: hoisted.
        let out = break_layout(src, "W", Drawn { rects: &rects }, &[]).unwrap();
        let c = check(&out);
        assert_eq!(c.root.children.len(), 2, "{out}");
        assert_eq!(c.at(&[1]).unwrap().num_prop("y"), Some(38.0));
        // The layout itself: becomes a Rectangle.
        let out = break_layout(src, "W", Drawn { rects: &rects }, &[0]).unwrap();
        let c = check(&out);
        let p = c.at(&[0]).unwrap();
        assert_eq!(p.kind, "Rectangle");
        assert!(p.prop("spacing").is_none());
        let b = p.children[1].clone();
        assert_eq!((b.num_prop("x"), b.num_prop("y")), (Some(0.0), Some(28.0)));
        assert_eq!(b.num_prop("width"), Some(280.0));
        assert_eq!(b.prop("width").unwrap().to_source(), "280px");
    }

    #[test]
    fn lay_out_a_rectangle_orders_children_by_position() {
        let src = "import { Button } from \"std-widgets.slint\";\nexport component W inherits Window {\n    Rectangle {\n        background: red;\n        low := Button { x: 0px; y: 100px; }\n        high := Button { x: 0px; y: 0px; }\n    }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0, 0], r(0.0, 100.0, 80.0, 24.0)),
            (vec![0, 1], r(0.0, 0.0, 80.0, 24.0)),
        ];
        let out = lay_out(src, "W", Drawn { rects: &rects }, &[0], Direction::Vertical).unwrap();
        let c = check(&out);
        let v = c.at(&[0]).unwrap();
        assert_eq!(v.kind, "VerticalBox");
        assert!(v.prop("background").is_none());
        let ids: Vec<_> = v.children.iter().map(|c| c.id.clone().unwrap()).collect();
        assert_eq!(ids, ["high", "low"]);
        assert!(v.children.iter().all(|c| c.prop("x").is_none()), "{out}");
        assert!(
            out.starts_with("import { Button, VerticalBox } from"),
            "{out}"
        );
    }

    #[test]
    fn lay_out_a_window_in_a_grid() {
        let src = "export component W inherits Window {\n    l2 := Text { x: 0px; y: 40px; }\n    l1 := Text { x: 0px; y: 0px; }\n    f1 := LineEdit { x: 80px; y: 0px; }\n    f2 := LineEdit { x: 80px; y: 42px; }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(0.0, 40.0, 60.0, 20.0)),
            (vec![1], r(0.0, 0.0, 60.0, 20.0)),
            (vec![2], r(80.0, 0.0, 160.0, 24.0)),
            (vec![3], r(80.0, 42.0, 160.0, 24.0)),
        ];
        let out = lay_out(src, "W", Drawn { rects: &rects }, &[], Direction::Grid).unwrap();
        let c = check(&out);
        let g = &c.root.children[0];
        assert_eq!(g.kind, "GridBox");
        let cells: Vec<_> = g
            .children
            .iter()
            .map(|c| (c.id.clone().unwrap(), c.num_prop("row"), c.num_prop("col")))
            .collect();
        assert_eq!(
            cells,
            [
                ("l1".into(), Some(0.0), Some(0.0)),
                ("f1".into(), Some(0.0), Some(1.0)),
                ("l2".into(), Some(1.0), Some(0.0)),
                ("f2".into(), Some(1.0), Some(1.0)),
            ],
            "{out}"
        );
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(13.0, true), 16.0);
        assert_eq!(snap(11.0, true), 8.0);
        assert_eq!(snap(-5.0, true), 0.0);
        assert_eq!(snap(13.4, false), 13.0);
    }
}

/// Align or distribute freely placed siblings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrange {
    Left,
    CenterH,
    Right,
    Top,
    CenterV,
    Bottom,
    /// Equal gaps left to right (3 or more).
    DistributeH,
    /// Equal gaps top to bottom (3 or more).
    DistributeV,
}

/// The common parent of `paths` if they're all freely placed in it.
pub fn common_free_parent(
    comp: &perox_slint::Component,
    paths: &[Vec<usize>],
) -> Option<Vec<usize>> {
    let parent = paths.first()?.split_last()?.1.to_vec();
    let same = paths
        .iter()
        .all(|p| p.split_last().is_some_and(|(_, q)| q == parent.as_slice()));
    let free = comp.at(&parent).is_some_and(|p| catalog::is_free(&p.kind));
    (same && free).then_some(parent)
}

/// Lines up `paths` (siblings in a free container) by their drawn rects. Alignment is
/// to the first path (the primary selection); distribution keeps the outer two where
/// they are.
pub fn arrange(
    src: &str,
    comp: &str,
    drawn: Drawn<'_>,
    paths: &[Vec<usize>],
    how: Arrange,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(src);
    let c = component(&doc, comp)?;
    let parent = common_free_parent(c, paths).ok_or(EditError::NoSuchElement)?;
    let origin = drawn.rect(&parent).ok_or(EditError::NoSuchElement)?.min;
    let mut items: Vec<(Vec<usize>, Rect)> = paths
        .iter()
        .filter_map(|p| {
            drawn
                .rect(p)
                .map(|r| (p.clone(), r.translate(-origin.to_vec2())))
        })
        .collect();
    if items.len() < 2 {
        return Err(EditError::NoSuchElement);
    }
    let reference = items[0].1;
    let mut moves: Vec<(Vec<usize>, Pos2)> = Vec::new();
    match how {
        Arrange::DistributeH | Arrange::DistributeV => {
            let horizontal = how == Arrange::DistributeH;
            let key = |r: &Rect| if horizontal { r.left() } else { r.top() };
            items.sort_by(|a, b| key(&a.1).total_cmp(&key(&b.1)));
            let len = |r: &Rect| if horizontal { r.width() } else { r.height() };
            let first = items.first().map(|i| key(&i.1)).unwrap_or(0.0);
            let last = items.last().map(|i| key(&i.1) + len(&i.1)).unwrap_or(0.0);
            let total: f32 = items.iter().map(|i| len(&i.1)).sum();
            let gap = (last - first - total) / (items.len() - 1) as f32;
            let mut at = first;
            for (p, r) in &items {
                let pos = if horizontal {
                    Pos2::new(at, r.top())
                } else {
                    Pos2::new(r.left(), at)
                };
                moves.push((p.clone(), pos));
                at += len(r) + gap;
            }
        }
        _ => {
            for (p, r) in &items {
                let x = match how {
                    Arrange::Left => reference.left(),
                    Arrange::CenterH => reference.center().x - r.width() / 2.0,
                    Arrange::Right => reference.right() - r.width(),
                    _ => r.left(),
                };
                let y = match how {
                    Arrange::Top => reference.top(),
                    Arrange::CenterV => reference.center().y - r.height() / 2.0,
                    Arrange::Bottom => reference.bottom() - r.height(),
                    _ => r.top(),
                };
                moves.push((p.clone(), Pos2::new(x, y)));
            }
        }
    }
    let mut work = src.to_owned();
    for (p, at) in moves {
        set_position(
            &mut work,
            comp,
            &p,
            Pos2::new(at.x.round().max(0.0), at.y.round().max(0.0)),
        )?;
    }
    Ok(work)
}

#[cfg(test)]
mod arrange_tests {
    use super::*;
    use eframe::egui::pos2;

    #[test]
    fn align_and_distribute() {
        let src = "export component W inherits Window {\n    a := Rectangle { x: 10px; y: 10px; }\n    b := Rectangle { x: 50px; y: 30px; }\n    c := Rectangle { x: 200px; y: 5px; }\n}\n";
        let r = |x: f32, y: f32, w: f32| Rect::from_min_size(pos2(x, y), vec2(w, 20.0));
        let rects = vec![
            (vec![], r(0.0, 0.0, 400.0)),
            (vec![0], r(10.0, 10.0, 20.0)),
            (vec![1], r(50.0, 30.0, 40.0)),
            (vec![2], r(200.0, 5.0, 20.0)),
        ];
        let all = [vec![0], vec![1], vec![2]];
        let out = arrange(src, "W", Drawn { rects: &rects }, &all, Arrange::Top).unwrap();
        assert_eq!(out.matches("y: 10px").count(), 3, "{out}");
        let out = arrange(
            src,
            "W",
            Drawn { rects: &rects },
            &[vec![1], vec![0]],
            Arrange::Right,
        )
        .unwrap();
        assert!(
            out.contains("a := Rectangle { x: 70px;"),
            "right edges at 90: {out}"
        );
        // 10..30, 50..90, 200..220: free space 210 - 80 = 130 → gaps of 65.
        let out = arrange(
            src,
            "W",
            Drawn { rects: &rects },
            &all,
            Arrange::DistributeH,
        )
        .unwrap();
        assert!(out.contains("b := Rectangle { x: 95px;"), "{out}");
        assert!(out.contains("c := Rectangle { x: 200px;"), "{out}");
    }
}
