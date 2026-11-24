//! Actions: one command used from several places, like a menu item and a toolbar button.
//!
//! Slint has no action object, so an action is two plain declarations on the component:
//!
//! ```slint
//! callback save();                               // what it does (a Rust handler)
//! in-out property <bool> save_enabled: true;     // optional: whether it can run now
//! ```
//!
//! An element using it forwards its trigger event to the callback and, if the action
//! can be disabled, binds `enabled: root.save_enabled;`. Rust calls
//! `ui.set_save_enabled(false)` once and every menu item and button for it greys out.

use perox_slint::edit::{self, EditError};
use perox_slint::{Component, Element};

use crate::{catalog, free, wire_event};

/// An action on a component, and who uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionInfo {
    pub name: String,
    /// `save_enabled`, if the action can be disabled.
    pub enabled: Option<String>,
    /// Elements whose trigger forwards to it.
    pub users: Vec<Vec<usize>>,
}

/// The property that enables `action`.
pub fn enabled_name(action: &str) -> String {
    format!("{action}_enabled")
}

/// The event that "runs" an element: `clicked`, `activated`, `triggered`… The first
/// parameterless event the catalog lists for it.
pub fn trigger(kind: &str) -> Option<&'static str> {
    catalog::events(kind)
        .iter()
        .find(|e| e.params.is_empty())
        .map(|e| e.name)
}

/// The component's actions: its callbacks without parameters or return value.
pub fn list(c: &Component) -> Vec<ActionInfo> {
    c.callbacks
        .iter()
        .filter(|cb| cb.params.is_empty() && cb.ret.is_none())
        .map(|cb| {
            let enabled = enabled_name(&cb.name);
            let mut users = Vec::new();
            c.root.walk(&mut Vec::new(), &mut |el: &Element, p| {
                // Only the element's trigger counts: a LineEdit whose `accepted(text)`
                // also greets mustn't be disabled with the action.
                let Some(event) = trigger(&el.kind) else {
                    return;
                };
                if el
                    .handler(event)
                    .and_then(|h| h.forwards_to())
                    .is_some_and(|f| same(f, &cb.name))
                {
                    users.push(p.to_vec());
                }
            });
            ActionInfo {
                name: cb.name.clone(),
                enabled: c.property(&enabled).map(|p| p.name.clone()),
                users,
            }
        })
        .collect()
}

/// Slint treats `-` and `_` in names as the same.
fn same(a: &str, b: &str) -> bool {
    a.replace('-', "_") == b.replace('-', "_")
}

/// Declares a new action. Returns the new source.
pub fn add(src: &str, comp: &str, name: &str, can_disable: bool) -> Result<String, EditError> {
    let mut work = src.to_owned();
    free::step(&mut work, edit::add_callback(src, comp, name, &[]))?;
    if can_disable {
        work = set_can_disable(&work, comp, name, true)?;
    }
    Ok(work)
}

/// Turns the action's `_enabled` property on or off, binding or unbinding `enabled` on
/// everything that uses it.
pub fn set_can_disable(src: &str, comp: &str, name: &str, on: bool) -> Result<String, EditError> {
    let prop = enabled_name(name);
    let mut work = src.to_owned();
    let doc = perox_slint::parse(src);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.to_owned()))?;
    let users = list(c)
        .into_iter()
        .find(|a| same(&a.name, name))
        .map(|a| a.users)
        .unwrap_or_default();
    if on {
        let s = work.clone();
        free::step(
            &mut work,
            edit::add_property(&s, comp, &prop, "bool", Some("true")),
        )?;
        for p in &users {
            let s = work.clone();
            free::step(
                &mut work,
                edit::set_property(&s, comp, p, "enabled", &format!("root.{prop}")),
            )?;
        }
    } else {
        for p in &users {
            let s = work.clone();
            let bound = perox_slint::parse(&s)
                .component(comp)
                .and_then(|c| c.at(p))
                .and_then(|e| e.prop("enabled"))
                .is_some_and(|v| same(&v.to_source(), &format!("root.{prop}")));
            if bound {
                free::step(&mut work, edit::remove_property(&s, comp, p, "enabled"))?;
            }
        }
        let s = work.clone();
        free::step(&mut work, edit::remove_property_decl(&s, comp, &prop))?;
    }
    Ok(work)
}

/// Makes the element at `path` run `action`: its trigger event forwards to it, and its
/// `enabled` follows the action's if it has one. Returns the new source.
pub fn use_action(
    src: &str,
    comp: &str,
    path: &[usize],
    action: &str,
) -> Result<String, EditError> {
    let doc = perox_slint::parse(src);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.to_owned()))?;
    let el = c.at(path).ok_or(EditError::NoSuchElement)?;
    let event = trigger(&el.kind).ok_or(EditError::NoSuchElement)?;
    let mut work = wire_event(src, comp, path, event, action)?;
    let prop = enabled_name(action);
    let has_enabled = perox_slint::parse(&work)
        .component(comp)
        .is_some_and(|c| c.property(&prop).is_some());
    if has_enabled {
        let s = work.clone();
        free::step(
            &mut work,
            edit::set_property(&s, comp, path, "enabled", &format!("root.{prop}")),
        )?;
    }
    Ok(work)
}

/// A caption for an action's name: `save_as` → `Save as`.
pub fn caption(name: &str) -> String {
    let words = name.replace(['_', '-'], " ");
    let mut c = words.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Adds a new element of `kind` (a `Button` in a toolbar, a `MenuItem` in a menu) as the
/// last child of `parent`, captioned after the action and using it. Returns the new
/// source and the element's path.
pub fn add_user(
    src: &str,
    comp: &str,
    parent: &[usize],
    kind: &str,
    action: &str,
) -> Result<(String, Vec<usize>), EditError> {
    let doc = perox_slint::parse(src);
    let c = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.to_owned()))?;
    let count = c.at(parent).ok_or(EditError::NoSuchElement)?.children.len();
    let text_prop = if kind == "MenuItem" { "title" } else { "text" };
    // `save_item`, `save_button`; numbered only if taken.
    let base = format!(
        "{}_{}",
        action.replace('-', "_"),
        catalog::snake(kind).rsplit('_').next().unwrap_or("item")
    );
    let id = if c.find(&base).is_none() {
        base
    } else {
        crate::unique_id(c, &base)
    };
    let props = vec![(
        text_prop.to_owned(),
        perox_slint::syntax::quote(&caption(action)),
    )];
    let mut work = src.to_owned();
    free::step(
        &mut work,
        edit::add_element(src, comp, parent, usize::MAX, kind, Some(&id), &props),
    )?;
    free::import(&mut work, kind);
    let mut path = parent.to_vec();
    path.push(count);
    let work = use_action(&work, comp, &path, action)?;
    Ok((work, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "import { Button, HorizontalBox, VerticalBox } from \"std-widgets.slint\";\nexport component Main inherits Window {\n    callback save();\n    callback edited(text: string);\n\n    MenuBar {\n        Menu {\n            title: \"File\";\n        }\n    }\n    VerticalBox {\n        toolbar := HorizontalBox { }\n    }\n}\n";

    fn ok(src: &str) -> perox_slint::Component {
        let d = perox_slint::parse(src);
        assert!(d.errors.is_empty(), "{src}\n{:?}", d.errors);
        d.components.into_iter().next().unwrap()
    }

    #[test]
    fn actions_are_parameterless_callbacks() {
        let a = list(&ok(SRC));
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].name, "save");
        assert_eq!(a[0].enabled, None);
        assert!(a[0].users.is_empty());
    }

    #[test]
    fn a_menu_item_and_a_button_share_an_action_and_its_enabled_flag() {
        let (s, item) = add_user(SRC, "Main", &[0, 0], "MenuItem", "save").unwrap();
        assert_eq!(item, vec![0, 0, 0]);
        let (s, button) = add_user(&s, "Main", &[1, 0], "Button", "save").unwrap();
        assert_eq!(button, vec![1, 0, 0]);
        assert!(s.contains("save_item := MenuItem {"), "{s}");
        assert!(s.contains("title: \"Save\";"), "{s}");
        assert!(s.contains("activated => { root.save(); }"), "{s}");
        // A LineEdit that also saves on Return isn't a user: it stays enabled.
        let s = s.replace(
            "toolbar := HorizontalBox { }",
            "toolbar := HorizontalBox { }\n        LineEdit { accepted(text) => { root.save(); } }",
        );
        assert!(s.contains("save_button := Button {"), "{s}");
        assert!(s.contains("clicked => { root.save(); }"), "{s}");
        assert_eq!(list(&ok(&s))[0].users, vec![item.clone(), button.clone()]);

        // Can be disabled: one property, both bound to it.
        let s = set_can_disable(&s, "Main", "save", true).unwrap();
        assert!(
            s.contains("in-out property <bool> save_enabled: true;"),
            "{s}"
        );
        assert_eq!(s.matches("enabled: root.save_enabled;").count(), 2, "{s}");
        assert!(
            s.contains(
                "save_button := Button { text: \"Save\"; enabled: root.save_enabled; clicked"
            ),
            "properties before handlers: {s}"
        );
        let c = ok(&s);
        assert_eq!(list(&c)[0].enabled.as_deref(), Some("save_enabled"));

        // A new user picks the binding up.
        let (s, _) = add_user(&s, "Main", &[1, 0], "Button", "save").unwrap();
        assert_eq!(s.matches("enabled: root.save_enabled;").count(), 3, "{s}");

        // And off again.
        let s = set_can_disable(&s, "Main", "save", false).unwrap();
        assert!(!s.contains("save_enabled"), "{s}");
        ok(&s);
    }

    #[test]
    fn new_actions_and_captions() {
        let s = add(SRC, "Main", "save_as", true).unwrap();
        assert!(s.contains("callback save_as();"), "{s}");
        assert!(
            s.contains("in-out property <bool> save_as_enabled: true;"),
            "{s}"
        );
        assert_eq!(caption("save_as"), "Save as");
        assert_eq!(trigger("Button"), Some("clicked"));
        assert_eq!(trigger("MenuItem"), Some("activated"));
        assert_eq!(trigger("Text"), None);
    }
}
