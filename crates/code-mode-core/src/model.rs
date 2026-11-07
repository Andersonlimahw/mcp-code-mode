use crate::InvokeError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One operation loaded from an OpenAPI document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub operation_id: String,
    pub binding_name: String,
    pub method: String,
    pub path: String,
    pub summary: String,
    pub description: String,
    pub tags: Vec<String>,
    pub parameters: Vec<Parameter>,
    pub request_schema: Value,
    pub response_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub location: ParameterLocation,
    pub required: bool,
    pub description: String,
    pub schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterLocation {
    Path,
    Query,
    Header,
    Cookie,
}

/// MCP tool definition. `input_schema` is JSON Schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Text payload returned from `tools/call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

/// Executes one indexed operation. The sandbox depends on this trait only.
pub trait OperationInvoker: Send + Sync {
    fn operations(&self) -> &[Operation];
    fn invoke(&self, operation_id: &str, args: &Value) -> Result<Value, InvokeError>;
}

/// Human-readable binding shown by `search`.
///
/// Path and query parameters are keys on the single argument map. The invoker
/// removes them before sending the JSON body.
pub fn binding_signature(operation: &Operation) -> String {
    let names: Vec<&str> = operation
        .parameters
        .iter()
        .filter(|parameter| {
            matches!(
                parameter.location,
                ParameterLocation::Path | ParameterLocation::Query
            )
        })
        .map(|parameter| parameter.name.as_str())
        .collect();
    if names.is_empty() {
        format!("{}(body)", operation.binding_name)
    } else {
        format!(
            "{}({{ {}, ...body }})",
            operation.binding_name,
            names.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn operation(binding: &str, params: Vec<Parameter>) -> Operation {
        Operation {
            operation_id: binding.to_string(),
            binding_name: binding.to_string(),
            method: "POST".into(),
            path: "/example".into(),
            summary: String::new(),
            description: String::new(),
            tags: Vec::new(),
            parameters: params,
            request_schema: json!({}),
            response_schema: json!({}),
        }
    }

    #[test]
    fn signature_without_params_is_body_only() {
        let op = operation("search", Vec::new());
        assert_eq!(binding_signature(&op), "search(body)");
    }

    #[test]
    fn signature_lists_path_and_query_names() {
        let op = operation(
            "update_page",
            vec![Parameter {
                name: "page_id".into(),
                location: ParameterLocation::Path,
                required: true,
                description: String::new(),
                schema: json!({"type": "string"}),
            }],
        );
        assert_eq!(binding_signature(&op), "update_page({ page_id, ...body })");
    }
}
