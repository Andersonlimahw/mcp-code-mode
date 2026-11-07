//! Sandboxed Rhai engine.
//!
//! Scripts call operation bindings (`create_page(#{ ... })`). The engine has
//! no filesystem, no network, and no `eval`. `print` is discarded so a script
//! cannot write to the MCP stdout stream. Only the final expression is returned.

mod convert;
mod engine;

pub use engine::{execute, ScriptLimits};
