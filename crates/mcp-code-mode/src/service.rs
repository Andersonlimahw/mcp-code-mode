use code_mode_adapters::{ApiAdapter, EndpointParts, HttpInvoker, HttpTransport};
use code_mode_core::{binding_signature, Clock, Operation, ToolDefinition, ToolResult};
use code_mode_index::{ensure_indexed, RankedOperation, SqliteStore};
use code_mode_sandbox::{execute, ScriptLimits};
use code_mode_transport::{ToolCallError, ToolProvider};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;

pub const INSTRUCTIONS: &str = "\
Call list_apis to see which APIs are loaded. \
Call search for compact operation signatures (id, method and path, summary, required parameters). \
Call describe with an operation_id when you need the full request and response schemas. \
Then call execute once. Bindings are namespaced as {adapter}_{operation} when more than one API is loaded. \
Path and query parameters are keys on the argument map and are not sent in the JSON body. \
Return only the final expression.";

const SCHEMA_CAP_BYTES: usize = 4_000;

struct LoadedApi {
    id: String,
    base_url: String,
}

pub struct CodeModeApp {
    apis: Vec<LoadedApi>,
    store: SqliteStore,
    invoker: Arc<dyn code_mode_core::OperationInvoker>,
    limits: ScriptLimits,
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Index(#[from] code_mode_index::IndexError),
    #[error(transparent)]
    Adapter(#[from] code_mode_adapters::AdapterError),
}

impl CodeModeApp {
    pub fn new(
        adapter: &dyn ApiAdapter,
        store: SqliteStore,
        http: Box<dyn HttpTransport>,
        clock: Arc<dyn Clock>,
        ttl_secs: i64,
        max_calls: u32,
        debug: bool,
    ) -> Result<Self, AppError> {
        Self::from_adapters(&[adapter], store, http, clock, ttl_secs, max_calls, debug)
    }

    pub fn from_adapters(
        adapters: &[&dyn ApiAdapter],
        store: SqliteStore,
        http: Box<dyn HttpTransport>,
        clock: Arc<dyn Clock>,
        ttl_secs: i64,
        max_calls: u32,
        debug: bool,
    ) -> Result<Self, AppError> {
        if adapters.is_empty() {
            return Err(AppError::Adapter(
                code_mode_adapters::AdapterError::Message("no adapters configured".into()),
            ));
        }
        let mut loaded_ops: Vec<(String, String, Vec<Operation>)> = Vec::new();
        let mut binding_counts: HashMap<String, usize> = HashMap::new();
        for adapter in adapters {
            let operations =
                ensure_indexed(&store, adapter.id(), adapter.spec_json(), clock.unix_secs())?;
            for operation in &operations {
                *binding_counts
                    .entry(operation.binding_name.clone())
                    .or_default() += 1;
            }
            loaded_ops.push((
                adapter.id().to_string(),
                adapter.base_url().to_string(),
                operations,
            ));
        }
        let multiple = loaded_ops.len() > 1;
        let mut endpoints = Vec::new();
        let mut apis = Vec::new();
        for (index, adapter) in adapters.iter().enumerate() {
            let (id, base_url, operations) = &loaded_ops[index];
            let expose_short =
                |name: &str| !multiple || binding_counts.get(name).copied() == Some(1);
            let runtime = runtime_operations(id, operations, expose_short);
            endpoints.push(EndpointParts {
                adapter_id: id.clone(),
                base_url: base_url.clone(),
                headers: adapter.headers()?,
                operations: runtime,
            });
            apis.push(LoadedApi {
                id: id.clone(),
                base_url: base_url.clone(),
            });
        }
        let invoker = Arc::new(HttpInvoker::from_endpoints(
            endpoints,
            http,
            store.clone(),
            Arc::clone(&clock),
            ttl_secs,
            max_calls,
            debug,
        ));
        Ok(Self {
            apis,
            store,
            invoker,
            limits: ScriptLimits::default(),
        })
    }

    pub fn adapter_id(&self) -> &str {
        self.apis
            .first()
            .map(|api| api.id.as_str())
            .unwrap_or("none")
    }

    pub fn adapter_ids(&self) -> Vec<&str> {
        self.apis.iter().map(|api| api.id.as_str()).collect()
    }

    pub fn list_apis_value(&self) -> Result<Value, String> {
        let mut apis = Vec::new();
        for api in &self.apis {
            let count = self
                .store
                .list_operations(&api.id)
                .map_err(|error| error.to_string())?
                .len();
            apis.push(json!({
                "id": api.id,
                "base_url": api.base_url,
                "operations": count
            }));
        }
        Ok(json!({"apis": apis}))
    }

    pub fn search_value(
        &self,
        query: &str,
        limit: usize,
        adapter: Option<&str>,
    ) -> Result<Value, String> {
        let hits = self.ranked(adapter, query, limit)?;
        let multiple = self.apis.len() > 1;
        let matches = hits
            .into_iter()
            .map(|(id, hit)| compact_match(&id, &hit, multiple))
            .collect::<Vec<_>>();
        let mut payload = Map::new();
        payload.insert("query".into(), Value::String(query.to_string()));
        if !multiple {
            if let Some(api) = self.apis.first() {
                payload.insert("adapter".into(), Value::String(api.id.clone()));
            }
        }
        payload.insert("matches".into(), Value::Array(matches));
        Ok(Value::Object(payload))
    }

    pub fn describe_value(
        &self,
        adapter: Option<&str>,
        operation_id: &str,
    ) -> Result<Value, String> {
        let (api_id, operation) = self.find_operation(adapter, operation_id)?;
        let multiple = self.apis.len() > 1;
        let mut display = operation.clone();
        display.binding_name = public_binding(&api_id, &operation, multiple);
        Ok(json!({
            "adapter": api_id,
            "operation_id": operation.operation_id,
            "binding": binding_signature(&display),
            "http": format!("{} {}", operation.method, operation.path),
            "summary": one_line(&operation.summary),
            "description": one_line(&operation.description),
            "parameters": operation.parameters,
            "request_schema": cap_schema(&operation.request_schema),
            "response_schema": cap_schema(&operation.response_schema)
        }))
    }

    fn ranked(
        &self,
        adapter: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, RankedOperation)>, String> {
        let ids = self.selected_ids(adapter)?;
        let mut hits = Vec::new();
        for id in ids {
            for hit in self
                .store
                .search(id, query, limit)
                .map_err(|error| error.to_string())?
            {
                hits.push((id.to_string(), hit));
            }
        }
        hits.sort_by(|left, right| {
            right
                .1
                .score
                .cmp(&left.1.score)
                .then_with(|| left.0.cmp(&right.0))
                .then_with(|| {
                    left.1
                        .operation
                        .operation_id
                        .cmp(&right.1.operation.operation_id)
                })
        });
        hits.truncate(limit);
        Ok(hits)
    }

    fn find_operation(
        &self,
        adapter: Option<&str>,
        operation_id: &str,
    ) -> Result<(String, Operation), String> {
        let ids = self.selected_ids(adapter)?;
        let mut found = Vec::new();
        for id in ids {
            for operation in self
                .store
                .list_operations(id)
                .map_err(|error| error.to_string())?
            {
                if operation.operation_id == operation_id || operation.binding_name == operation_id
                {
                    found.push((id.to_string(), operation));
                }
            }
        }
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(format!("unknown operation `{operation_id}`")),
            _ => Err(format!(
                "operation `{operation_id}` is in more than one API; pass adapter"
            )),
        }
    }

    fn selected_ids(&self, adapter: Option<&str>) -> Result<Vec<&str>, String> {
        if let Some(adapter) = adapter {
            let Some(api) = self.apis.iter().find(|api| api.id == adapter) else {
                return Err(format!("unknown adapter `{adapter}`"));
            };
            return Ok(vec![api.id.as_str()]);
        }
        Ok(self.apis.iter().map(|api| api.id.as_str()).collect())
    }
}

fn runtime_operations(
    adapter_id: &str,
    operations: &[Operation],
    expose_short: impl Fn(&str) -> bool,
) -> Vec<Operation> {
    let mut runtime = Vec::new();
    for operation in operations {
        let key = format!("{adapter_id}::{}", operation.operation_id);
        let mut prefixed = operation.clone();
        prefixed.operation_id = key.clone();
        prefixed.binding_name = format!("{adapter_id}_{}", operation.binding_name);
        runtime.push(prefixed);
        if expose_short(&operation.binding_name) {
            let mut short = operation.clone();
            short.operation_id = key;
            runtime.push(short);
        }
    }
    runtime
}

fn public_binding(adapter_id: &str, operation: &Operation, multiple: bool) -> String {
    if multiple {
        format!("{adapter_id}_{}", operation.binding_name)
    } else {
        operation.binding_name.clone()
    }
}

fn compact_match(adapter_id: &str, hit: &RankedOperation, multiple: bool) -> Value {
    let operation = &hit.operation;
    let mut display = operation.clone();
    display.binding_name = public_binding(adapter_id, operation, multiple);
    let mut payload = Map::new();
    if multiple {
        payload.insert("adapter".into(), Value::String(adapter_id.to_string()));
    }
    payload.insert(
        "operation_id".into(),
        Value::String(operation.operation_id.clone()),
    );
    payload.insert("binding".into(), Value::String(binding_signature(&display)));
    payload.insert(
        "http".into(),
        Value::String(format!("{} {}", operation.method, operation.path)),
    );
    payload.insert(
        "summary".into(),
        Value::String(one_line(&operation.summary)),
    );
    let required = required_params(operation);
    if !required.is_empty() {
        payload.insert("required".into(), Value::Array(required));
    }
    Value::Object(payload)
}

fn required_params(operation: &Operation) -> Vec<Value> {
    let mut required = Vec::new();
    for parameter in &operation.parameters {
        if !parameter.required {
            continue;
        }
        let location = match parameter.location {
            code_mode_core::ParameterLocation::Path => "path",
            code_mode_core::ParameterLocation::Query => "query",
            code_mode_core::ParameterLocation::Header
            | code_mode_core::ParameterLocation::Cookie => continue,
        };
        required.push(Value::String(format!(
            "{}:{location}:{}",
            parameter.name,
            schema_type(&parameter.schema)
        )));
    }
    let Some(names) = operation
        .request_schema
        .get("required")
        .and_then(Value::as_array)
    else {
        return required;
    };
    let properties = operation
        .request_schema
        .get("properties")
        .and_then(Value::as_object);
    for name in names {
        let Some(name) = name.as_str() else {
            continue;
        };
        let schema = properties
            .and_then(|map| map.get(name))
            .cloned()
            .unwrap_or(Value::Null);
        required.push(Value::String(format!(
            "{name}:body:{}",
            schema_type(&schema)
        )));
    }
    required
}

fn schema_type(schema: &Value) -> String {
    match schema.get("type") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("|"),
        _ if schema.get("properties").is_some() => "object".into(),
        _ if schema.get("items").is_some() => "array".into(),
        _ => "any".into(),
    }
}

fn one_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= 120 {
        return line.to_string();
    }
    let mut end = 0;
    for (index, _) in line.char_indices().take(117) {
        end = index;
    }
    let mut boundary = end;
    while boundary > 0 && !line.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &line[..boundary])
}

impl ToolProvider for CodeModeApp {
    fn instructions(&self) -> &str {
        INSTRUCTIONS
    }

    fn list_tools(&self) -> Vec<ToolDefinition> {
        tool_definitions()
    }

    fn call_tool(&self, name: &str, arguments: Value) -> Result<ToolResult, ToolCallError> {
        match name {
            "list_apis" => Ok(ok_json(
                self.list_apis_value().map_err(ToolCallError::Invalid)?,
            )),
            "search" => {
                let query = arguments
                    .get("query")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|query| !query.is_empty())
                    .ok_or_else(|| {
                        ToolCallError::Invalid("search requires a query string".into())
                    })?;
                let limit = read_limit(&arguments)?;
                let adapter = optional_string(&arguments, "adapter")?;
                Ok(ok_json(
                    self.search_value(query, limit, adapter.as_deref())
                        .map_err(ToolCallError::Invalid)?,
                ))
            }
            "describe" => {
                let operation_id = arguments
                    .get("operation_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        ToolCallError::Invalid("describe requires operation_id".into())
                    })?;
                let adapter = optional_string(&arguments, "adapter")?;
                Ok(ok_json(
                    self.describe_value(adapter.as_deref(), operation_id)
                        .map_err(ToolCallError::Invalid)?,
                ))
            }
            "execute" => {
                let code = arguments
                    .get("code")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ToolCallError::Invalid("execute requires a code string".into())
                    })?;
                match execute(code, Arc::clone(&self.invoker), &self.limits) {
                    Ok(value) => Ok(ToolResult {
                        content: render_value(&value),
                        is_error: false,
                    }),
                    Err(error) => Ok(ToolResult {
                        content: error.to_string(),
                        is_error: true,
                    }),
                }
            }
            other => Err(ToolCallError::UnknownTool(other.to_string())),
        }
    }
}

fn ok_json(value: Value) -> ToolResult {
    ToolResult {
        content: serde_json::to_string(&value).unwrap_or_else(|_| "null".into()),
        is_error: false,
    }
}

fn optional_string(arguments: &Value, name: &str) -> Result<Option<String>, ToolCallError> {
    match arguments.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if text.trim().is_empty() => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.trim().to_string())),
        _ => Err(ToolCallError::Invalid(format!("{name} must be a string"))),
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "list_apis".into(),
            description: "List the APIs loaded in this server: id, base URL, and operation count.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "search".into(),
            description: "Search indexed operations. Returns compact signatures: id, HTTP method and path, one-line summary, and required parameters with types. Use describe for full schemas.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keywords such as \"create page\" or \"list projects\"."
                    },
                    "adapter": {
                        "type": "string",
                        "description": "Limit results to one API id from list_apis."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Maximum matches. Defaults to 5."
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "describe".into(),
            description: "Return the full parameter list and request/response schemas for one operation. Pass adapter when the id exists in more than one API.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "operation_id": {
                        "type": "string",
                        "description": "Operation id or binding name from search."
                    },
                    "adapter": {
                        "type": "string",
                        "description": "API id. Required if operation_id is not unique."
                    }
                },
                "required": ["operation_id"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "execute".into(),
            description: "Run a Rhai script. Bindings come from search. With one API they are unprefixed (create_page). With several APIs they are {adapter}_{binding} (notion_create_page). Only the final expression is returned.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "code": {
                        "type": "string",
                        "description": "Rhai script. The last expression is the result."
                    }
                },
                "required": ["code"],
                "additionalProperties": false
            }),
        },
    ]
}

fn read_limit(arguments: &Value) -> Result<usize, ToolCallError> {
    match arguments.get("limit") {
        None | Some(Value::Null) => Ok(5),
        Some(Value::Number(number)) => {
            let number = number
                .as_u64()
                .ok_or_else(|| ToolCallError::Invalid("limit must be an integer".into()))?;
            if (1..=20).contains(&number) {
                Ok(usize::try_from(number).unwrap_or(20))
            } else {
                Err(ToolCallError::Invalid(
                    "limit must be between 1 and 20".into(),
                ))
            }
        }
        _ => Err(ToolCallError::Invalid("limit must be an integer".into())),
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
    }
}

fn cap_schema(value: &Value) -> Value {
    if value.is_null() {
        return Value::Null;
    }
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "{}".into());
    if raw.len() <= SCHEMA_CAP_BYTES {
        return value.clone();
    }
    let mut end = SCHEMA_CAP_BYTES.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    json!({
        "truncated": true,
        "bytes": raw.len(),
        "preview": &raw[..end]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{McpServer, NotionAdapter, OpenApiAdapter, VERSION};
    use code_mode_adapters::HttpTransport;
    use code_mode_core::SystemClock;
    use code_mode_index::SqliteStore;

    struct DenyHttp;

    impl HttpTransport for DenyHttp {
        fn send(
            &self,
            _request: &code_mode_adapters::HttpRequest,
        ) -> Result<code_mode_adapters::HttpResponse, code_mode_adapters::HttpError> {
            Err(code_mode_adapters::HttpError::Message(
                "search must not call the network".into(),
            ))
        }
    }

    fn app() -> CodeModeApp {
        let adapter = NotionAdapter::new("token", "https://api.notion.com", "2022-06-28").unwrap();
        CodeModeApp::new(
            &adapter,
            SqliteStore::open_in_memory().unwrap(),
            Box::new(DenyHttp),
            Arc::new(SystemClock),
            60,
            8,
            false,
        )
        .unwrap()
    }

    #[test]
    fn pilot_search_is_compact() {
        let app = app();
        let value = app.search_value("create database", 3, None).unwrap();
        let hit = &value["matches"][0];
        assert_eq!(hit["operation_id"], "create_database");
        assert_eq!(hit["binding"], "create_database(body)");
        assert_eq!(hit["http"], "POST /v1/databases");
        assert!(hit.get("request_schema").is_none());
        assert!(hit.get("description").is_none());
        let required = hit["required"].as_array().unwrap();
        let rendered: Vec<_> = required.iter().map(|item| item.as_str().unwrap()).collect();
        assert!(rendered.contains(&"parent:body:object"));
        assert!(rendered.contains(&"properties:body:object"));
    }

    #[test]
    fn describe_returns_schemas() {
        let app = app();
        let value = app.describe_value(None, "create_page").unwrap();
        assert!(value["request_schema"]["properties"]["parent"].is_object());
        assert_eq!(value["http"], "POST /v1/pages");
    }

    #[test]
    fn page_database_query_covers_the_workflow() {
        let app = app();
        let value = app.search_value("page database", 5, None).unwrap();
        let ids: Vec<_> = value["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| hit["operation_id"].as_str().unwrap().to_string())
            .collect();
        for required in [
            "search",
            "create_page",
            "update_page",
            "create_database",
            "update_database",
        ] {
            assert!(
                ids.iter().any(|id| id == required),
                "{ids:?} missing {required}"
            );
        }
    }

    #[test]
    fn tools_are_list_search_describe_execute() {
        let server = McpServer::new(app(), "mcp-code-mode", VERSION);
        let response = server
            .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
            .unwrap();
        let names: Vec<_> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["list_apis", "search", "describe", "execute"]);
    }

    #[test]
    fn two_adapters_prefix_bindings() {
        let notion = NotionAdapter::new("token", "https://api.notion.com", "2022-06-28").unwrap();
        let other = OpenApiAdapter::new(
            "demo",
            "https://example.test",
            r#"{"openapi":"3.0.3","info":{"title":"Demo","version":"1"},"paths":{"/ping":{"get":{"operationId":"ping","summary":"Ping the demo API"}}}}"#,
            Some("demo-token".into()),
            Vec::new(),
        )
        .unwrap();
        let app = CodeModeApp::from_adapters(
            &[&notion, &other],
            SqliteStore::open_in_memory().unwrap(),
            Box::new(DenyHttp),
            Arc::new(SystemClock),
            0,
            8,
            false,
        )
        .unwrap();
        let listed = app.list_apis_value().unwrap();
        assert_eq!(listed["apis"].as_array().unwrap().len(), 2);
        let value = app.search_value("create page", 3, Some("notion")).unwrap();
        assert_eq!(value["matches"][0]["adapter"], "notion");
        assert!(value["matches"][0]["binding"]
            .as_str()
            .unwrap()
            .starts_with("notion_"));
        let ping = app.search_value("ping", 1, Some("demo")).unwrap();
        assert_eq!(ping["matches"][0]["binding"], "demo_ping(body)");
    }
}
