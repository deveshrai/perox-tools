//! Where a dragged widget lands: which container, at which child index, and the line
//! the canvas draws to show it. Pure functions over the laid-out rects.

use eframe::egui::{Pos2, Rect, pos2};
use perox_forms::File;

use crate::catalog;

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub parent: Vec<usize>,
    /// Child index as before any move.
    pub index: usize,
    /// The insertion marker, in screen points.
    pub line: [Pos2; 2],
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Axis {
    Vertical,
    Horizontal,
    Reading,
}

fn axis(kind: &str) -> Axis {
    match kind {
        "HBox" => Axis::Horizontal,
        "Grid" => Axis::Reading,
        _ => Axis::Vertical,
    }
}

/// The drop target for `child_kind` at `pointer`. `moving` is the widget being moved
/// (it and its children can't be targets).
pub fn target(
    file: &File,
    rects: &[(Vec<usize>, Rect)],
    pointer: Pos2,
    child_kind: &str,
    moving: Option<&[usize]>,
) -> Option<Target> {
    let excluded = |p: &[usize]| moving.is_some_and(|m| p.starts_with(m));
    // Deepest element under the pointer, then up to the nearest container that takes it.
    let hit = rects
        .iter()
        .rev()
        .find(|(p, r)| r.contains(pointer) && !excluded(p))
        .map(|(p, _)| p.clone())?;
    let mut parent = hit;
    loop {
        let el = file.at(&parent)?;
        if catalog::accepts(&el.kind, child_kind) {
            break;
        }
        parent.pop()?;
        if parent.is_empty() && !catalog::accepts(&file.root()?.kind, child_kind) {
            return None;
        }
    }
    let el = file.at(&parent)?;
    let rect_of = |p: &[usize]| rects.iter().find(|(q, _)| q == p).map(|(_, r)| *r);
    let parent_rect = rect_of(&parent)?;
    // Visible children (a hidden tab page has no rect), minus the one being moved.
    let children: Vec<(usize, Rect)> = (0..el.children.len())
        .filter_map(|i| {
            let mut p = parent.clone();
            p.push(i);
            if excluded(&p) {
                return None;
            }
            rect_of(&p).map(|r| (i, r))
        })
        .collect();
    let ax = axis(&el.kind);
    let before = |r: &Rect| match ax {
        Axis::Vertical => pointer.y < r.center().y,
        Axis::Horizontal => pointer.x < r.center().x,
        Axis::Reading => {
            pointer.y < r.top() || (pointer.y <= r.bottom() && pointer.x < r.center().x)
        }
    };
    let next = children.iter().find(|(_, r)| before(r));
    let index = next.map_or(el.children.len(), |(i, _)| *i);
    let inner = parent_rect.shrink(2.0);
    let line = match (next, children.last(), ax) {
        (Some((_, r)), _, Axis::Vertical) => horizontal(inner, r.top() - 3.0),
        (None, Some((_, r)), Axis::Vertical) => horizontal(inner, r.bottom() + 3.0),
        (Some((_, r)), _, _) => vertical(r.left() - 3.0, *r),
        (None, Some((_, r)), _) => vertical(r.right() + 3.0, *r),
        (None, None, Axis::Horizontal) => vertical(inner.left() + 2.0, inner),
        (None, None, _) => horizontal(inner, inner.top() + 2.0),
    };
    Some(Target {
        parent,
        index,
        line,
    })
}

fn horizontal(r: Rect, y: f32) -> [Pos2; 2] {
    [pos2(r.left(), y), pos2(r.right(), y)]
}

fn vertical(x: f32, r: Rect) -> [Pos2; 2] {
    [pos2(x, r.top()), pos2(x, r.bottom())]
}

/// True if moving `from` to (`parent`, `index`) would leave it where it is.
pub fn is_noop(from: &[usize], parent: &[usize], index: usize) -> bool {
    let (last, from_parent) = match from.split_last() {
        Some((l, p)) => (*l, p),
        None => return true,
    };
    from_parent == parent && (index == last || index == last + 1)
}

/// The moved widget's path once `from` is moved to (`parent`, `index`), given indices
/// as before the move.
pub fn moved_path(from: &[usize], parent: &[usize], index: usize) -> Vec<usize> {
    let (&from_last, from_parent) = from.split_last().expect("not the root");
    let mut new_parent = parent.to_vec();
    // Removing `from` shifts later siblings (and anything under them) back by one.
    let d = from_parent.len();
    if new_parent.len() > d && new_parent[..d] == *from_parent && new_parent[d] > from_last {
        new_parent[d] -= 1;
    }
    let mut index = index;
    if parent == from_parent && index > from_last {
        index -= 1;
    }
    new_parent.push(index);
    new_parent
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn file() -> File {
        perox_forms::parse(
            "Window w {\n VBox {\n Button a {}\n HBox {\n Button b {}\n Button c {}\n }\n Label l {}\n }\n}",
        )
    }

    fn rects() -> Vec<(Vec<usize>, Rect)> {
        vec![
            (vec![], r(0.0, 0.0, 300.0, 300.0)),
            (vec![0], r(10.0, 10.0, 280.0, 280.0)),
            (vec![0, 0], r(10.0, 10.0, 280.0, 20.0)),
            (vec![0, 1], r(10.0, 40.0, 280.0, 20.0)),
            (vec![0, 1, 0], r(10.0, 40.0, 100.0, 20.0)),
            (vec![0, 1, 1], r(120.0, 40.0, 100.0, 20.0)),
            (vec![0, 2], r(10.0, 70.0, 280.0, 20.0)),
        ]
    }

    #[test]
    fn dropping_on_a_leaf_inserts_beside_it_in_its_parent() {
        let f = file();
        // Upper half of `a`: before it.
        let t = target(&f, &rects(), pos2(50.0, 12.0), "Label", None).unwrap();
        assert_eq!((t.parent, t.index), (vec![0], 0));
        // Lower half of `l`: after it.
        let t = target(&f, &rects(), pos2(50.0, 88.0), "Label", None).unwrap();
        assert_eq!((t.parent, t.index), (vec![0], 3));
        // Right half of `b` in the row: between b and c.
        let t = target(&f, &rects(), pos2(100.0, 50.0), "Label", None).unwrap();
        assert_eq!((t.parent, t.index), (vec![0, 1], 1));
        // Empty space under everything: end of the VBox.
        let t = target(&f, &rects(), pos2(50.0, 200.0), "Label", None).unwrap();
        assert_eq!((t.parent, t.index), (vec![0], 3));
    }

    #[test]
    fn a_widget_cannot_land_inside_itself() {
        let f = file();
        let t = target(&f, &rects(), pos2(100.0, 50.0), "HBox", Some(&[0, 1])).unwrap();
        assert_eq!(t.parent, vec![0], "falls through to the VBox");
    }

    #[test]
    fn tab_pages_only_go_into_tabs() {
        let f = file();
        assert!(target(&f, &rects(), pos2(50.0, 12.0), "Tab", None).is_none());
    }

    #[test]
    fn move_paths() {
        assert!(is_noop(&[0, 1], &[0], 1));
        assert!(is_noop(&[0, 1], &[0], 2));
        assert!(!is_noop(&[0, 1], &[0], 3));
        assert_eq!(moved_path(&[0, 0], &[0], 3), vec![0, 2]);
        assert_eq!(moved_path(&[0, 2], &[0], 0), vec![0, 0]);
        // Out of [0,0] into the HBox at [0,1], which becomes [0,0].
        assert_eq!(moved_path(&[0, 0], &[0, 1], 1), vec![0, 0, 1]);
        assert_eq!(moved_path(&[0, 1, 0], &[0], 0), vec![0, 0]);
    }
}
