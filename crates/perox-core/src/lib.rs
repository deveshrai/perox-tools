//! PeroxIDE core. Start with [`command`]: every IDE action is a command on
//! the [`CommandBus`], and menus, shortcuts, the palette and MCP all call it.

pub mod command;

pub use command::{Access, CommandBus, CommandError, CommandSpec, Invocation, Origin};
