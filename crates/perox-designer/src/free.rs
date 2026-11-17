//! Free placement: converting between layouts and `x`/`y` geometry, like Qt's
//! "Break Layout" and "Lay Out Vertically / Horizontally / in a Grid".
//!
//! Each operation runs several edits on a working copy of the source and returns the
//! final text; the designer turns that into one minimal edit (one undo step).

use eframe::egui::{Pos2, Rect, Vec2, vec2};
use perox_forms::edit::{self, EditError, TextEdit};
use perox_forms::{File, Value};

/// Apply one step to the working copy.
pub fn step(work: &mut String, edits: Result<Vec<TextEdit>, EditError>) -> Result<(), EditError> {
    let edits = edits?;
    *work = edit::apply(work, &edits);
    Ok(())
}

/// Where things were drawn: element rects and content boxes (same order).
#[derive(Debug, Clone, Copy)]
pub struct Drawn<'a> {
    pub rects: &'a [(Vec<usize>, Rect)],
    pub contents: &'a [Rect],
}

impl Drawn<'_> {
    pub fn rect(&self, path: &[usize]) -> Option<Rect> {
        self.rects.iter().find(|(p, _)| p == path).map(|(_, r)| *r)
    }

    pub fn content(&self, path: &[usize]) -> Option<Rect> {
        let i = self.rects.iter().position(|(p, _)| p == path)?;
        self.contents.get(i).copied()
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
            Direction::Vertical => "VBox",
            Direction::Horizontal => "HBox",
            Direction::Grid => "Grid",
        }
    }
}

/// Snap to the grid (when on) and keep inside the container (no negative positions).
pub fn snap(v: f32, on: bool) -> f32 {
    let v = v.max(0.0);
    if on {
        (v / perox_ui::GRID).round() * perox_ui::GRID
    } else {
        v.round()
    }
}

fn num(v: f32) -> Value {
    Value::Num(v.round() as f64)
}

/// Write `x`, `y`, `width`, `height` for the element at `path`.
pub fn set_geometry(work: &mut String, path: &[usize], r: Rect) -> Result<(), EditError> {
    for (name, v) in [
        ("x", r.min.x),
        ("y", r.min.y),
        ("width", r.width()),
        ("height", r.height()),
    ] {
        let src = work.clone();
        step(work, edit::set_property(&src, path, name, &num(v)))?;
    }
    Ok(())
}

pub fn set_position(work: &mut String, path: &[usize], at: Pos2) -> Result<(), EditError> {
    for (name, v) in [("x", at.x), ("y", at.y)] {
        let src = work.clone();
        step(work, edit::set_property(&src, path, name, &num(v)))?;
    }
    Ok(())
}

/// Remove properties that only mean something in the old arrangement.
pub fn strip(work: &mut String, path: &[usize], names: &[&str]) -> Result<(), EditError> {
    for name in names {
        let src = work.clone();
        step(work, edit::remove_property(&src, path, name))?;
    }
    Ok(())
}

const GEOMETRY: &[&str] = &["x", "y", "width", "height"];
const LAYOUT_CHILD: &[&str] = &["row", "column", "row_span", "column_span", "stretch"];
const LAYOUT_SELF: &[&str] = &["spacing", "align", "justify", "columns"];

fn is_layout(kind: &str) -> bool {
    matches!(kind, "VBox" | "HBox" | "Grid")
}

/// Where `x: 0, y: 0` will be once the container at `path` is free.
fn free_origin(file: &File, drawn: Drawn<'_>, path: &[usize]) -> Option<Pos2> {
    let el = file.at(path)?;
    let rect = drawn.rect(path)?;
    if el.kind == "GroupBox" {
        // Same padding in both modes.
        return drawn.content(path).map(|c| c.min);
    }
    let margin = el.num_prop("margin").unwrap_or(0.0) as f32;
    Some(rect.min + Vec2::splat(margin))
}

fn relative(r: Rect, origin: Pos2) -> Rect {
    r.translate(-origin.to_vec2())
}

/// "Break Layout" on the element at `path`. A `VBox`/`HBox`/`Grid` becomes a `Panel`;
/// a `Window`, `GroupBox` or `Tab` gets `layout: free` (taking over the children of
/// its single layout, if that's what it holds). Every widget keeps its place on
/// screen.
pub fn break_layout(src: &str, drawn: Drawn<'_>, path: &[usize]) -> Result<String, EditError> {
    let file = perox_forms::parse(src);
    let el = file.at(path).ok_or(EditError::NoSuchElement)?;
    let mut work = src.to_owned();
    if is_layout(&el.kind) {
        let origin = drawn.content(path).ok_or(EditError::NoSuchElement)?.min;
        let geometry: Vec<(usize, Rect)> = child_rects(drawn, path, el.children.len(), origin);
        step(&mut work, edit::set_kind(src, path, "Panel"))?;
        strip(&mut work, path, LAYOUT_SELF)?;
        for (i, r) in geometry {
            let mut p = path.to_vec();
            p.push(i);
            strip(&mut work, &p, LAYOUT_CHILD)?;
            set_geometry(&mut work, &p, r)?;
        }
        drop_spacers(&mut work, path)?;
        return Ok(work);
    }
    if !matches!(el.kind.as_str(), "Window" | "GroupBox" | "Tab") || perox_ui::is_free(el) {
        return Err(EditError::NoSuchElement);
    }
    let origin = free_origin(&file, drawn, path).ok_or(EditError::NoSuchElement)?;
    let single_layout = el.children.len() == 1 && is_layout(&el.children[0].kind);
    if single_layout {
        // Hoist the layout's children into the container, then drop the layout.
        let mut lp = path.to_vec();
        lp.push(0);
        let inner = &el.children[0];
        let geometry = child_rects(drawn, &lp, inner.children.len(), origin);
        for j in (0..inner.children.len()).rev() {
            let mut from = lp.clone();
            from.push(j);
            let s = work.clone();
            step(&mut work, edit::move_widget(&s, &from, path, 1))?;
        }
        let s = work.clone();
        step(&mut work, edit::remove_widget(&s, &lp))?;
        let s = work.clone();
        step(
            &mut work,
            edit::set_property(&s, path, "layout", &Value::Ident("free".into())),
        )?;
        for (j, r) in geometry {
            let mut p = path.to_vec();
            p.push(j);
            strip(&mut work, &p, LAYOUT_CHILD)?;
            set_geometry(&mut work, &p, r)?;
        }
    } else {
        let geometry = child_rects(drawn, path, el.children.len(), origin);
        step(
            &mut work,
            edit::set_property(src, path, "layout", &Value::Ident("free".into())),
        )?;
        for (i, r) in geometry {
            let mut p = path.to_vec();
            p.push(i);
            strip(&mut work, &p, LAYOUT_CHILD)?;
            set_geometry(&mut work, &p, r)?;
        }
    }
    drop_spacers(&mut work, path)?;
    Ok(work)
}

/// Spacers only mean something in a layout; free placement drops them (as Qt does).
fn drop_spacers(work: &mut String, path: &[usize]) -> Result<(), EditError> {
    let file = perox_forms::parse(work);
    let Some(el) = file.at(path) else {
        return Ok(());
    };
    let spacers: Vec<usize> = (0..el.children.len())
        .filter(|i| el.children[*i].kind == "Spacer")
        .collect();
    for i in spacers.into_iter().rev() {
        let mut p = path.to_vec();
        p.push(i);
        let s = work.clone();
        step(work, edit::remove_widget(&s, &p))?;
    }
    Ok(())
}

/// Children's rects relative to `origin`, for the children that were drawn.
fn child_rects(drawn: Drawn<'_>, parent: &[usize], n: usize, origin: Pos2) -> Vec<(usize, Rect)> {
    (0..n)
        .filter_map(|i| {
            let mut p = parent.to_vec();
            p.push(i);
            drawn.rect(&p).map(|r| (i, relative(r, origin)))
        })
        .collect()
}

/// Reading order for a grid: rows by `y` (a new row starts below the current row's
/// top by more than half a typical height), then `x`. Returns child indexes and the
/// column count.
fn reading_order(rects: &[(usize, Rect)], dir: Direction) -> (Vec<usize>, usize) {
    let mut items: Vec<(usize, Rect)> = rects.to_vec();
    match dir {
        Direction::Vertical => {
            items.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
            (items.iter().map(|(i, _)| *i).collect(), 1)
        }
        Direction::Horizontal => {
            items.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
            let n = items.len();
            (items.iter().map(|(i, _)| *i).collect(), n)
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
            let cols = rows.iter().map(Vec::len).max().unwrap_or(1);
            let order = rows
                .into_iter()
                .flat_map(|mut row| {
                    row.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
                    row.into_iter().map(|(i, _)| i)
                })
                .collect();
            (order, cols)
        }
    }
}

/// "Lay Out Vertically / Horizontally / in a Grid" on a free container at `path`,
/// ordering children by where they sit. A `Panel` becomes the layout; a free
/// `Window`, `GroupBox` or `Tab` gets a new layout holding its children.
pub fn lay_out(
    src: &str,
    drawn: Drawn<'_>,
    path: &[usize],
    dir: Direction,
) -> Result<String, EditError> {
    let file = perox_forms::parse(src);
    let el = file.at(path).ok_or(EditError::NoSuchElement)?;
    if !perox_ui::is_free(el) {
        return Err(EditError::NoSuchElement);
    }
    let n = el.children.len();
    let origin = Pos2::ZERO;
    let mut rects = child_rects(drawn, path, n, origin);
    // Hidden children have no rect; keep them, at the end.
    for i in 0..n {
        if !rects.iter().any(|(j, _)| *j == i) {
            rects.push((
                i,
                Rect::from_min_size(Pos2::new(f32::MAX / 4.0, f32::MAX / 4.0), vec2(1.0, 1.0)),
            ));
        }
    }
    let (order, cols) = reading_order(&rects, dir);
    let mut work = src.to_owned();
    // The layout the children go into, and where they are now.
    let (layout, mut current, offset): (Vec<usize>, Vec<usize>, usize) = if el.kind == "Panel" {
        (path.to_vec(), (0..n).collect(), 0)
    } else {
        step(
            &mut work,
            edit::add_widget(src, path, 0, dir.kind(), None, &[]),
        )?;
        let mut lp = path.to_vec();
        lp.push(0);
        (lp, (0..n).collect(), 1)
    };
    for (k, child) in order.iter().enumerate() {
        let pos = current
            .iter()
            .position(|c| c == child)
            .expect("each child once");
        let s = work.clone();
        if el.kind == "Panel" {
            // Reorder in place.
            if pos != k {
                let mut from = path.to_vec();
                from.push(pos);
                step(&mut work, edit::move_widget(&s, &from, path, k))?;
                let c = current.remove(pos);
                current.insert(k, c);
            }
        } else {
            let mut from = path.to_vec();
            from.push(offset + pos);
            step(&mut work, edit::move_widget(&s, &from, &layout, k))?;
            current.remove(pos);
        }
    }
    for k in 0..n {
        let mut p = layout.clone();
        p.push(k);
        strip(&mut work, &p, GEOMETRY)?;
    }
    if el.kind == "Panel" {
        let s = work.clone();
        step(&mut work, edit::set_kind(&s, path, dir.kind()))?;
        strip(&mut work, path, &["frame"])?;
    } else {
        strip(&mut work, path, &["layout"])?;
    }
    if dir == Direction::Grid {
        let s = work.clone();
        let columns = Value::Num(cols.max(1) as f64);
        step(
            &mut work,
            edit::set_property(&s, &layout, "columns", &columns),
        )?;
    }
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    #[test]
    fn break_a_vbox_into_a_panel_keeping_positions() {
        let src = "Window w {\n    VBox {\n        spacing: 4\n        Button a { text: \"A\" }\n        Button b { text: \"B\" }\n    }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(10.0, 10.0, 280.0, 180.0)),
            (vec![0, 0], r(10.0, 10.0, 280.0, 24.0)),
            (vec![0, 1], r(10.0, 38.0, 280.0, 24.0)),
        ];
        let contents: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let out = break_layout(
            src,
            Drawn {
                rects: &rects,
                contents: &contents,
            },
            &[0],
        )
        .unwrap();
        let f = perox_forms::parse(&out);
        assert!(f.errors.is_empty(), "{out}");
        let p = f.at(&[0]).unwrap();
        assert_eq!(p.kind, "Panel");
        assert!(p.prop("spacing").is_none());
        let b = f.at(&[0, 1]).unwrap();
        assert_eq!((b.num_prop("x"), b.num_prop("y")), (Some(0.0), Some(28.0)));
        assert_eq!(b.num_prop("width"), Some(280.0));
    }

    #[test]
    fn break_a_window_layout_hoists_its_children() {
        let src = "Window w {\n    VBox {\n        Button a {}\n        Button b {}\n        Spacer {}\n    }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(10.0, 10.0, 280.0, 180.0)),
            (vec![0, 0], r(10.0, 10.0, 280.0, 24.0)),
            (vec![0, 1], r(10.0, 40.0, 100.0, 24.0)),
        ];
        let contents: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let out = break_layout(
            src,
            Drawn {
                rects: &rects,
                contents: &contents,
            },
            &[],
        )
        .unwrap();
        let f = perox_forms::parse(&out);
        assert!(f.errors.is_empty(), "{out}");
        let w = f.root().unwrap();
        assert_eq!(w.str_prop("layout"), Some("free"));
        let ids: Vec<_> = w.children.iter().map(|c| c.id.clone().unwrap()).collect();
        assert_eq!(ids, ["a", "b"], "the spacer is gone: {out}");
        assert!(!out.contains("Spacer"), "{out}");
        // A free window has no margin, so x/y are window coordinates.
        assert_eq!(w.children[1].num_prop("x"), Some(10.0));
        assert_eq!(w.children[1].num_prop("y"), Some(40.0));
    }

    #[test]
    fn lay_out_a_panel_orders_children_by_position() {
        let src = "Window w {\n    Panel {\n        Button low { x: 0; y: 100 }\n        Button high { x: 0; y: 0 }\n    }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0, 0], r(0.0, 100.0, 80.0, 24.0)),
            (vec![0, 1], r(0.0, 0.0, 80.0, 24.0)),
        ];
        let contents: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let out = lay_out(
            src,
            Drawn {
                rects: &rects,
                contents: &contents,
            },
            &[0],
            Direction::Vertical,
        )
        .unwrap();
        let f = perox_forms::parse(&out);
        assert!(f.errors.is_empty(), "{out}");
        let v = f.at(&[0]).unwrap();
        assert_eq!(v.kind, "VBox");
        let ids: Vec<_> = v.children.iter().map(|c| c.id.clone().unwrap()).collect();
        assert_eq!(ids, ["high", "low"]);
        assert!(
            v.children
                .iter()
                .all(|c| c.prop("x").is_none() && c.prop("y").is_none()),
            "{out}"
        );
    }

    #[test]
    fn lay_out_a_free_window_in_a_grid() {
        let src = "Window w {\n    layout: free\n    Label l2 { x: 0; y: 40 }\n    Label l1 { x: 0; y: 0 }\n    TextEdit f1 { x: 80; y: 0 }\n    TextEdit f2 { x: 80; y: 42 }\n}\n";
        let rects = vec![
            (vec![], r(0.0, 0.0, 300.0, 200.0)),
            (vec![0], r(0.0, 40.0, 60.0, 20.0)),
            (vec![1], r(0.0, 0.0, 60.0, 20.0)),
            (vec![2], r(80.0, 0.0, 160.0, 24.0)),
            (vec![3], r(80.0, 42.0, 160.0, 24.0)),
        ];
        let contents: Vec<Rect> = rects.iter().map(|(_, r)| *r).collect();
        let out = lay_out(
            src,
            Drawn {
                rects: &rects,
                contents: &contents,
            },
            &[],
            Direction::Grid,
        )
        .unwrap();
        let f = perox_forms::parse(&out);
        assert!(f.errors.is_empty(), "{out}");
        let w = f.root().unwrap();
        assert!(w.prop("layout").is_none());
        let g = &w.children[0];
        assert_eq!(g.kind, "Grid");
        assert_eq!(g.num_prop("columns"), Some(2.0));
        let ids: Vec<_> = g.children.iter().map(|c| c.id.clone().unwrap()).collect();
        assert_eq!(ids, ["l1", "f1", "l2", "f2"], "{out}");
    }

    #[test]
    fn snapping() {
        assert_eq!(snap(13.0, true), 16.0);
        assert_eq!(snap(11.0, true), 8.0);
        assert_eq!(snap(-5.0, true), 0.0);
        assert_eq!(snap(13.4, false), 13.0);
    }
}
