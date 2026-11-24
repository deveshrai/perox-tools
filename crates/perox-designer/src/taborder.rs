//! Tab order. Slint moves keyboard focus through the widgets in the order they appear in
//! the `.slint` file. In a container that places children by `x`/`y`, that order can
//! change without anything moving on screen; in a layout, the order *is* the layout, so
//! changing one changes the other.

use perox_slint::Component;
use perox_slint::edit;

use crate::catalog;

/// Widgets that take keyboard focus.
pub fn focusable(kind: &str) -> bool {
    matches!(
        kind,
        "Button"
            | "StandardButton"
            | "LineEdit"
            | "TextEdit"
            | "CheckBox"
            | "Switch"
            | "ComboBox"
            | "SpinBox"
            | "Slider"
            | "StandardListView"
            | "StandardTableView"
            | "TextInput"
            | "FocusScope"
            | "TabWidget"
            | "DatePickerPopup"
            | "TimePickerPopup"
    )
}

/// The focusable widgets in tab order (source order).
pub fn order(c: &Component) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    c.root.walk(&mut Vec::new(), &mut |el, p| {
        if focusable(&el.kind) {
            out.push(p.to_vec());
        }
    });
    out
}

/// Moves the widget at `path` one place earlier (`up`) or later in the tab order, by
/// moving it in the source past its neighbour in the order. Only within one freely placed
/// container, where nothing moves on screen. Returns the new source and its new path.
pub fn step(
    src: &str,
    comp: &str,
    path: &[usize],
    up: bool,
) -> Result<(String, Vec<usize>), String> {
    let doc = perox_slint::parse(src);
    let c = doc.component(comp).ok_or("no such component")?;
    let list = order(c);
    let i = list
        .iter()
        .position(|p| p == path)
        .ok_or("That widget doesn't take keyboard focus")?;
    let j = if up {
        i.checked_sub(1).ok_or("It's first already")?
    } else if i + 1 < list.len() {
        i + 1
    } else {
        return Err("It's last already".into());
    };
    let other = &list[j];
    let (Some((_, parent)), Some((&b_idx, other_parent))) = (path.split_last(), other.split_last())
    else {
        return Err("The window itself isn't in the tab order".into());
    };
    if parent != other_parent {
        return Err(
            "Tab order goes through containers in turn: move the widget into the same container first"
                .into(),
        );
    }
    let parent_kind = c.at(parent).map(|e| e.kind.clone()).unwrap_or_default();
    if !catalog::is_free(&parent_kind) {
        return Err(format!(
            "In a {parent_kind} the tab order is the layout's order: drag the widget to where it should come, or use Break layout to place things freely"
        ));
    }
    // Moving before `other` when going up; after it when going down.
    let to = if up { b_idx } else { b_idx + 1 };
    let edits = edit::move_element(src, comp, path, parent, to).map_err(|e| e.to_string())?;
    let out = edit::apply(src, &edits);
    // Either way it ends up where its neighbour was.
    let mut new_path = parent.to_vec();
    new_path.push(b_idx);
    Ok((out, new_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREE: &str = "import { Button, LineEdit } from \"std-widgets.slint\";\nexport component W inherits Window {\n    name := LineEdit { x: 10px; y: 10px; }\n    Text { text: \"x\"; }\n    ok := Button { x: 10px; y: 50px; }\n    email := LineEdit { x: 10px; y: 30px; }\n}\n";

    fn ids(src: &str) -> Vec<String> {
        let doc = perox_slint::parse(src);
        let c = &doc.components[0];
        order(c)
            .iter()
            .map(|p| c.at(p).unwrap().id.clone().unwrap_or_default())
            .collect()
    }

    #[test]
    fn reorders_freely_placed_widgets_without_moving_them() {
        assert_eq!(ids(FREE), ["name", "ok", "email"]);
        // email up: before ok.
        let (s, p) = step(FREE, "W", &[3], true).unwrap();
        assert_eq!(ids(&s), ["name", "email", "ok"]);
        assert_eq!(p, vec![2]);
        assert!(s.contains("email := LineEdit { x: 10px; y: 30px; }"), "{s}");
        // name down: after email.
        let (s, p) = step(&s, "W", &[0], false).unwrap();
        assert_eq!(ids(&s), ["email", "name", "ok"]);
        let doc = perox_slint::parse(&s);
        assert_eq!(
            doc.components[0].at(&p).unwrap().id.as_deref(),
            Some("name")
        );
        assert!(step(&s, "W", &p, true).is_ok());
        assert!(step(FREE, "W", &[0], true).unwrap_err().contains("first"));
    }

    #[test]
    fn layouts_keep_their_order() {
        let src = "import { Button, LineEdit, VerticalBox } from \"std-widgets.slint\";\nexport component W inherits Window {\n    VerticalBox {\n        a := LineEdit { }\n        b := Button { }\n    }\n}\n";
        let e = step(src, "W", &[0, 1], true).unwrap_err();
        assert!(e.contains("layout's order"), "{e}");
    }
}
