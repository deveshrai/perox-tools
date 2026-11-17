//! PeroxIDE's code editor: text buffer, cursors, editing operations, syntax
//! highlighting, completion and the egui widget. See the crate README.

pub mod completion;
pub mod core;
pub mod selection;
pub mod snippet;
pub mod syntax;
pub mod text;
pub mod theme;
pub mod view;

pub use crate::core::EditorCore;
pub use selection::{Selection, Selections};
pub use text::{Change, Document, Edited, Pos};
pub use view::{Decorations, Diagnostic, EditorState, Output, Request, Severity, Signature};
