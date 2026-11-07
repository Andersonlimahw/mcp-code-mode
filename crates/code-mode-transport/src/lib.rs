//! MCP stdio transport.
//!
//! Frames are newline-delimited JSON-RPC 2.0. Logs stay off stdout; only
//! response messages are written there.

mod server;

pub use server::{serve_stdio, McpServer, ToolCallError, ToolProvider};

use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("parse error: {0}")]
    Parse(String),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message.into()}
    })
}
