//! The `.pxui` form language. See the crate README and ADR 0003.

pub mod codegen;
pub mod edit;
pub mod syntax;

pub use syntax::{Element, File, Handler, ParseError, Property, Value, parse, parse_value};
