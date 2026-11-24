//! The Translations window: every `@tr` string of the form's folder against each
//! language, editable in a table, saved as `.po` files ([`perox_slint::i18n`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};
use perox_slint::edit::{self, EditError};
use perox_slint::i18n::{self, Msg, Po};
use perox_slint::{Component, Value};

/// Properties that hold text a person reads.
pub const TEXT_PROPS: &[&str] = &["text", "title", "placeholder-text"];

/// Wraps every plain string in `comp`'s text properties in `@tr(…)`. Returns the new
/// source and how many it changed.
pub fn make_translatable(src: &str, comp: &str) -> Result<(String, usize), EditError> {
    let doc = perox_slint::parse(src);
    let c: &Component = doc
        .component(comp)
        .ok_or_else(|| EditError::NoSuchComponent(comp.to_owned()))?;
    let mut targets: Vec<(Vec<usize>, String, String)> = Vec::new();
    c.root.walk(&mut Vec::new(), &mut |el, p| {
        for prop in TEXT_PROPS {
            if let Some(Value::Str(s)) = el.prop(prop)
                && !s.trim().is_empty()
            {
                targets.push((p.to_vec(), (*prop).to_owned(), s.clone()));
            }
        }
    });
    let mut work = src.to_owned();
    for (path, prop, text) in &targets {
        let s = work.clone();
        crate::free::step(
            &mut work,
            edit::set_property(&s, comp, path, prop, &i18n::tr(text)),
        )?;
    }
    Ok((work, targets.len()))
}

/// Where a form's translations are: its `ui/` folder (the one with
/// `slint-project.json`, else the form's own) and the gettext domain (the crate name).
pub fn location(form_dir: &Path) -> (PathBuf, String) {
    let ui = perox_slint::project::find(form_dir)
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| form_dir.to_owned());
    let domain = i18n::domain_for(&ui).unwrap_or_else(|| "app".into());
    (ui, domain)
}

#[derive(Debug, Default)]
pub struct Editor {
    pub open: bool,
    ui: PathBuf,
    domain: String,
    msgs: Vec<Msg>,
    po: BTreeMap<String, Po>,
    new_lang: String,
    dirty: bool,
    pub status: Option<String>,
}

impl Editor {
    /// Opens the window on the translations of the form in `form_dir`, re-reading the
    /// strings and files.
    pub fn open(&mut self, form_dir: &Path) {
        let (ui, domain) = location(form_dir);
        self.ui = ui;
        self.domain = domain;
        self.reload();
        self.open = true;
    }

    fn reload(&mut self) {
        self.msgs = i18n::extract_dir(&self.ui);
        self.po = i18n::languages(&self.ui)
            .into_iter()
            .map(|l| {
                let po = std::fs::read_to_string(i18n::po_path(&self.ui, &l, &self.domain))
                    .map(|t| i18n::parse_po(&t))
                    .unwrap_or_default();
                (l, po)
            })
            .collect();
        self.dirty = false;
    }

    /// Writes every language's `.po` (adding new strings, dropping gone ones).
    pub fn save(&mut self) -> std::io::Result<()> {
        for (lang, po) in &self.po {
            let path = i18n::po_path(&self.ui, lang, &self.domain);
            if let Some(d) = path.parent() {
                std::fs::create_dir_all(d)?;
            }
            std::fs::write(&path, i18n::write_po(lang, &self.msgs, po))?;
        }
        self.dirty = false;
        Ok(())
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = true;
        egui::Window::new("Translations")
            .open(&mut open)
            .default_size([640.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Add language");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.new_lang)
                            .hint_text("de, fr, pt_BR")
                            .desired_width(80.0),
                    );
                    let tag = self.new_lang.trim().to_owned();
                    let ok = i18n::is_language_tag(&tag) && !self.po.contains_key(&tag);
                    if ui.add_enabled(ok, egui::Button::new("Add")).clicked() {
                        self.po.insert(tag, Po::default());
                        self.new_lang.clear();
                        self.dirty = true;
                    }
                    ui.separator();
                    if ui.button("Reload").clicked() {
                        self.reload();
                    }
                    if ui
                        .add_enabled(self.dirty, egui::Button::new("Save"))
                        .clicked()
                    {
                        self.status = Some(match self.save() {
                            Ok(()) => "Saved. Rebuild to see them in the app.".into(),
                            Err(e) => format!("Couldn't save: {e}"),
                        });
                    }
                    if let Some(s) = &self.status {
                        ui.weak(s);
                    }
                });
                ui.weak(format!(
                    "{} strings in {}; files in {}/<language>/LC_MESSAGES/{}.po",
                    self.msgs.len(),
                    self.ui.display(),
                    i18n::DIR,
                    self.domain
                ));
                if self.msgs.is_empty() {
                    ui.label("No translatable text yet: turn on the translate toggle next to a text property, or use Make All Text Translatable.");
                    return;
                }
                ui.separator();
                let langs: Vec<String> = self.po.keys().cloned().collect();
                egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
                    egui::Grid::new("translations")
                        .striped(true)
                        .num_columns(langs.len() + 2)
                        .show(ui, |ui| {
                            ui.strong("Form");
                            ui.strong("Text");
                            for l in &langs {
                                ui.strong(l);
                            }
                            ui.end_row();
                            for m in &self.msgs {
                                ui.label(RichText::new(&m.context).weak());
                                ui.label(&m.id);
                                for l in &langs {
                                    let po = self.po.get_mut(l).expect("listed");
                                    let mut t = po.get(&m.context, &m.id).unwrap_or("").to_owned();
                                    if ui
                                        .add(egui::TextEdit::singleline(&mut t).desired_width(140.0))
                                        .changed()
                                    {
                                        po.set(&m.context, &m.id, &t);
                                        self.dirty = true;
                                    }
                                }
                                ui.end_row();
                            }
                        });
                });
            });
        if !open {
            self.open = false;
        }
    }

    pub fn languages(&self) -> Vec<String> {
        self.po.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_text_becomes_translatable() {
        let src = "import { Button } from \"std-widgets.slint\";\nexport component W inherits Window {\n    title: \"Notes\";\n    Button { text: \"Save\"; }\n    Text { text: root.title; }\n    Text { text: \"\"; }\n    Text { text: @tr(\"Done\"); }\n}\n";
        let (out, n) = make_translatable(src, "W").unwrap();
        assert_eq!(n, 2);
        assert!(out.contains("title: @tr(\"Notes\");"), "{out}");
        assert!(out.contains("Button { text: @tr(\"Save\"); }"), "{out}");
        assert!(
            out.contains("text: root.title;") && out.contains("text: \"\";"),
            "{out}"
        );
        let (_, again) = make_translatable(&out, "W").unwrap();
        assert_eq!(again, 0);
    }
}
