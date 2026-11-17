//! PeroxIDE's language-server client. `client` speaks JSON-RPC over a child process's
//! stdio without ever blocking the UI; `rust_analyzer` starts the server with the
//! capabilities the editor uses. Converting LSP positions to editor positions lives in
//! the app, next to the documents.

pub mod client;
pub mod rust_analyzer;
pub mod uri;

pub use client::{Client, Event};
pub use rust_analyzer::RustAnalyzer;
