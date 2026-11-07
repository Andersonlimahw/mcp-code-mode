//! Composition root for the Code Mode MCP server.
//!
//! `search` reads the SQLite spec index. `execute` runs one Rhai script whose
//! bindings are the indexed operations. Intermediate HTTP bodies stay in the
//! sandbox; the tool result is the script's final value.

mod receipt;
mod service;

pub use code_mode_adapters::{
    ApiAdapter, GitHubAdapter, HttpError, HttpRequest, HttpResponse, HttpTransport, NotionAdapter,
    OpenApiAdapter, SupabaseAdapter, UreqTransport, VercelAdapter,
};
pub use code_mode_core::{Clock, SystemClock, ToolResult};
pub use code_mode_index::SqliteStore;
pub use code_mode_transport::{serve_stdio, McpServer, ToolProvider};
pub use receipt::render_receipt;
pub use service::{tool_definitions, AppError, CodeModeApp, INSTRUCTIONS};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
