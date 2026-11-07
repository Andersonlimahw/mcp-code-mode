use crate::{error_response, TransportError};
use code_mode_core::{ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use thiserror::Error;

const SUPPORTED_PROTOCOLS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
const DEFAULT_PROTOCOL: &str = "2024-11-05";

pub trait ToolProvider {
    fn instructions(&self) -> &str;
    fn list_tools(&self) -> Vec<ToolDefinition>;
    fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolResult, ToolCallError>;
}

#[derive(Debug, Error)]
pub enum ToolCallError {
    #[error("unknown tool `{0}`")]
    UnknownTool(String),
    #[error("{0}")]
    Invalid(String),
}

pub struct McpServer<P> {
    provider: P,
    server_name: String,
    server_version: String,
}

impl<P> McpServer<P> {
    pub fn new(
        provider: P,
        server_name: impl Into<String>,
        server_version: impl Into<String>,
    ) -> Self {
        Self {
            provider,
            server_name: server_name.into(),
            server_version: server_version.into(),
        }
    }
}

impl<P: ToolProvider> McpServer<P> {
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle_message(&message),
            Err(error) => Some(error_response(
                Value::Null,
                -32700,
                format!("parse error: {error}"),
            )),
        }
    }

    pub fn handle_message(&self, message: &Value) -> Option<Value> {
        let Some(message) = message.as_object() else {
            return Some(error_response(Value::Null, -32600, "invalid request"));
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let Some(method) = method else {
            return id.map(|id| error_response(id, -32600, "invalid request"));
        };
        if id.is_none() || method.starts_with("notifications/") {
            return None;
        }
        let id = id.unwrap_or(Value::Null);
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let result = match method {
            "initialize" => self.initialize(&params),
            "ping" => json!({}),
            "tools/list" => json!({"tools": self.tools_value()}),
            "tools/call" => match self.call_tool(&params) {
                Ok(value) => value,
                Err(error) => {
                    return Some(error_response(id, -32602, error.to_string()));
                }
            },
            "resources/list" => json!({"resources": []}),
            "resources/templates/list" => json!({"resourceTemplates": []}),
            "prompts/list" => json!({"prompts": []}),
            _ => {
                return Some(error_response(
                    id,
                    -32601,
                    format!("method not found: {method}"),
                ))
            }
        };
        Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }))
    }

    fn initialize(&self, params: &Value) -> Value {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL);
        let protocol = if SUPPORTED_PROTOCOLS.contains(&requested) {
            requested
        } else {
            DEFAULT_PROTOCOL
        };
        json!({
            "protocolVersion": protocol,
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": self.server_name,
                "version": self.server_version
            },
            "instructions": self.provider.instructions()
        })
    }

    fn tools_value(&self) -> Vec<Value> {
        self.provider
            .list_tools()
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "inputSchema": tool.input_schema
                })
            })
            .collect()
    }

    fn call_tool(&self, params: &Value) -> Result<Value, ToolCallError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolCallError::Invalid("tools/call requires name".into()))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let result = self.provider.call_tool(name, arguments)?;
        Ok(json!({
            "content": [{"type": "text", "text": result.content}],
            "isError": result.is_error
        }))
    }
}

pub fn serve_stdio<P: ToolProvider>(
    provider: P,
    server_name: &str,
    server_version: &str,
) -> Result<(), TransportError> {
    let server = McpServer::new(provider, server_name, server_version);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            serde_json::to_writer(&mut stdout, &response).map_err(std::io::Error::other)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TwoTools;

    impl ToolProvider for TwoTools {
        fn instructions(&self) -> &str {
            "search then execute"
        }

        fn list_tools(&self) -> Vec<ToolDefinition> {
            vec![
                ToolDefinition {
                    name: "search".into(),
                    description: "find operations".into(),
                    input_schema: json!({"type": "object"}),
                },
                ToolDefinition {
                    name: "execute".into(),
                    description: "run a script".into(),
                    input_schema: json!({"type": "object"}),
                },
            ]
        }

        fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolResult, ToolCallError> {
            match name {
                "search" => Ok(ToolResult {
                    content: arguments.to_string(),
                    is_error: false,
                }),
                "execute" => Ok(ToolResult {
                    content: "oops".into(),
                    is_error: true,
                }),
                _ => Err(ToolCallError::UnknownTool(name.into())),
            }
        }
    }

    fn server() -> McpServer<TwoTools> {
        McpServer::new(TwoTools, "mcp-code-mode", "0.1.0")
    }

    #[test]
    fn initialize_and_tools_list_expose_two_tools() {
        let server = server();
        let init = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#)
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "mcp-code-mode");
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        assert!(server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());
        let listed = server
            .handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "search");
        assert_eq!(tools[1]["name"], "execute");
    }

    #[test]
    fn tool_error_stays_inside_the_result() {
        let server = server();
        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"execute","arguments":{"code":"1"}}}"#)
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["text"], "oops");
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let server = server();
        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":"x","method":"tools/call","params":{"name":"delete_all"}}"#)
            .unwrap();
        assert_eq!(response["id"], "x");
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn parse_error_has_a_response() {
        let server = server();
        let response = server.handle_line("not json").unwrap();
        assert_eq!(response["error"]["code"], -32700);
    }
}
