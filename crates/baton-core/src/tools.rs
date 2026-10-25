//! Built-in tool boundary: catalog, project activation, and isolated execution.
//!
//! Definitions and activation are deliberately separate from runtime execution so
//! prompts cannot create tools or expand permissions.
mod catalog;
mod runtime;

pub use catalog::{Definition, IDS, ToolError, definition, definitions, disable, enable, list};
pub use runtime::execute;
