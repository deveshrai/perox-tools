//! Open documents: each is a file with an editor.

use std::path::{Path, PathBuf};

use perox_editor::EditorState;
use perox_editor::syntax::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocId(pub u64);

/// A `.slint` file, which opens in the designer.
pub fn is_form(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "slint")
}

#[derive(Debug)]
pub struct OpenDoc {
    pub id: DocId,
    pub path: PathBuf,
    pub editor: EditorState,
    /// `.slint` files: the designer, and whether it's showing (else the source is).
    pub designer: Option<Box<perox_designer::Designer>>,
    pub design_view: bool,
}

impl OpenDoc {
    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map_or_else(|| "untitled".into(), |n| n.to_string_lossy().into_owned());
        if self.editor.core.doc.is_dirty() {
            format!("● {name}")
        } else {
            name
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.editor.core.doc.is_dirty()
    }
}

#[derive(Debug, Default)]
pub struct Docs {
    pub list: Vec<OpenDoc>,
    next: u64,
}

impl Docs {
    pub fn by_path(&self, path: &Path) -> Option<DocId> {
        self.list.iter().find(|d| d.path == path).map(|d| d.id)
    }

    pub fn get(&self, id: DocId) -> Option<&OpenDoc> {
        self.list.iter().find(|d| d.id == id)
    }

    pub fn get_mut(&mut self, id: DocId) -> Option<&mut OpenDoc> {
        self.list.iter_mut().find(|d| d.id == id)
    }

    /// Opens a file (or returns the already-open one). `true` if newly opened.
    pub fn open(&mut self, path: &Path) -> Result<(DocId, bool), String> {
        if let Some(id) = self.by_path(path) {
            return Ok((id, false));
        }
        let bytes =
            std::fs::read(path).map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| format!("{} isn't UTF-8 text.", path.display()))?;
        self.next += 1;
        let id = DocId(self.next);
        self.list.push(OpenDoc {
            id,
            path: path.to_owned(),
            editor: EditorState::new(&text, Lang::from_path(path)),
            designer: is_form(path).then(|| Box::new(perox_designer::Designer::new())),
            design_view: is_form(path),
        });
        Ok((id, true))
    }

    pub fn save(&mut self, id: DocId) -> Result<PathBuf, String> {
        let doc = self.get_mut(id).ok_or("No such document.")?;
        std::fs::write(&doc.path, doc.editor.core.text())
            .map_err(|e| format!("Couldn't save {}: {e}", doc.path.display()))?;
        doc.editor.core.doc.mark_saved();
        Ok(doc.path.clone())
    }

    pub fn close(&mut self, id: DocId) -> Option<OpenDoc> {
        let i = self.list.iter().position(|d| d.id == id)?;
        Some(self.list.remove(i))
    }
}
