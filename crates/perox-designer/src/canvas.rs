//! The Canvas palette item: an `Image` that shows what the Rust code draws (with
//! `perox_ui::canvas`), for plots, charts, games, anything a layout of widgets can't.
//!
//! Dropping one named `plot` adds, besides the element:
//!
//! ```slint
//! in property <image> plot_image;             // what the code draws
//! out property <length> plot_width: plot.width;
//! out property <length> plot_height: plot.height;
//! callback plot_resized();                    // draw again at the new size
//! plot := Image {
//!     source: root.plot_image;
//!     image-fit: fill;
//!     changed width => { root.plot_resized(); }
//!     changed height => { root.plot_resized(); }
//! }
//! ```

use perox_slint::edit;

/// After the `Image` named `id` was added to `comp`: its declarations and wiring.
pub fn wire(src: &str, comp: &str, id: &str) -> Result<String, edit::EditError> {
    // Each step edits the text the one before made.
    fn step(
        work: &mut String,
        f: impl FnOnce(&str) -> Result<Vec<edit::TextEdit>, edit::EditError>,
    ) -> Result<(), edit::EditError> {
        let e = f(work)?;
        *work = edit::apply(work, &e);
        Ok(())
    }
    let mut work = src.to_owned();
    step(&mut work, |w| {
        edit::declare_property(w, comp, "in", &format!("{id}_image"), "image", None)
    })?;
    for side in ["width", "height"] {
        step(&mut work, |w| {
            edit::declare_property(
                w,
                comp,
                "out",
                &format!("{id}_{side}"),
                "length",
                Some(&format!("{id}.{side}")),
            )
        })?;
    }
    step(&mut work, |w| {
        edit::add_callback(w, comp, &format!("{id}_resized"), &[])
    })?;
    let path = perox_slint::parse(&work)
        .component(comp)
        .and_then(|c| c.find(id))
        .ok_or(edit::EditError::NoSuchElement)?;
    step(&mut work, |w| {
        edit::set_property(w, comp, &path, "source", &format!("root.{id}_image"))
    })?;
    for side in ["width", "height"] {
        step(&mut work, |w| {
            edit::set_handler(
                w,
                comp,
                &path,
                &format!("changed {side}"),
                &[],
                &format!("root.{id}_resized();"),
            )
        })?;
    }
    Ok(work)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canvas_is_wired_for_drawing() {
        let src = "export component Main inherits Window {\n    VerticalBox {\n        plot := Image { image-fit: fill; }\n    }\n}\n";
        let out = wire(src, "Main", "plot").unwrap();
        let doc = perox_slint::parse(&out);
        assert!(doc.errors.is_empty(), "{out}\n{:?}", doc.errors);
        assert!(out.contains("in property <image> plot_image;"), "{out}");
        assert!(out.contains("out property <length> plot_width: plot.width;"));
        assert!(out.contains("callback plot_resized();"));
        assert!(out.contains("source: root.plot_image;"));
        assert!(out.contains("changed height => { root.plot_resized(); }"));
    }
}
