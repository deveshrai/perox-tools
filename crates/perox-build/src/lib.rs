//! Running cargo for the IDE. `tools` finds the toolchain's programs; `job` runs a cargo
//! command and turns its JSON output into diagnostics and Output-panel lines.

pub mod job;
pub mod tools;

pub use job::{Diagnostic, Event, Job, Kind, Level};
