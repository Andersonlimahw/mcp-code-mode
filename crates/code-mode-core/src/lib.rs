//! Types shared by the transport, index, sandbox, and adapters.
//!
//! Crates depend on this one inward. It does not depend on HTTP, SQLite, or Rhai.

mod clock;
mod error;
mod model;

pub use clock::{Clock, ManualClock, SystemClock};
pub use error::{InvokeError, SandboxError};
pub use model::{
    binding_signature, Operation, OperationInvoker, Parameter, ParameterLocation, ToolDefinition,
    ToolResult,
};
