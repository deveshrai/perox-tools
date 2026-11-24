//! The `.slint` documents PeroxIDE's designer edits (ADR 0011). See the crate README.

pub mod codegen;
pub mod edit;
pub mod syntax;

pub use syntax::{
    Callback, Component, Document, Element, Handler, Import, ParseError, Property, Value, parse,
};

/// For `build.rs`: `perox_slint::build::compile("ui")`.
#[cfg(feature = "build")]
pub use codegen::build;
