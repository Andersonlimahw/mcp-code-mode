use crate::http::{cache_key, credential_fingerprint, HttpRequest, HttpTransport};
use code_mode_core::{Clock, InvokeError, Operation, OperationInvoker, ParameterLocation};
use code_mode_index::{CachedHttp, SqliteStore};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// HTTP operation client used by the sandbox.
///
/// GET responses are stored in the SQLite TTL cache. Mutations always hit the
/// network, so a script cannot replay a stale write.
///
/// Runtime operation ids are `{adapter_id}::{operation_id}` when more than one
/// API is mounted. A single endpoint still accepts the bare operation id.
pub struct HttpInvoker {
    endpoints: HashMap<String, Endpoint>,
    operations: Vec<Operation>,
    http: Box<dyn HttpTransport>,
    cache: SqliteStore,
    clock: Arc<dyn Clock>,
    ttl_secs: i64,
    calls: AtomicU32,
    max_calls: u32,
    debug: bool,
}

struct Endpoint {
    adapter_id: String,
    base_url: String,
    headers: Vec<(String, String)>,
    fingerprint: String,
}

pub struct InvokerParts {
    pub adapter_id: String,
    pub base_url: String,
    pub headers: Vec<(String, String)>,
    pub operations: Vec<Operation>,
    pub ttl_secs: i64,
    pub max_calls: u32,
    pub debug: bool,
}

/// One API inside a multi-adapter invoker. The caller namespaces operation ids.
pub struct EndpointParts {
    pub adapter_id: String,
    pub base_url: String,
    pub headers: Vec<(String, String)>,
    pub operations: Vec<Operation>,
}

impl HttpInvoker {
    pub fn new(
        parts: InvokerParts,
        http: Box<dyn HttpTransport>,
        cache: SqliteStore,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self::from_endpoints(
            vec![EndpointParts {
                adapter_id: parts.adapter_id,
                base_url: parts.base_url,
                headers: parts.headers,
                operations: parts.operations,
            }],
            http,
            cache,
            clock,
            parts.ttl_secs,
            parts.max_calls,
            parts.debug,
        )
    }

    pub fn from_endpoints(
        endpoints: Vec<EndpointParts>,
        http: Box<dyn HttpTransport>,
        cache: SqliteStore,
        clock: Arc<dyn Clock>,
        ttl_secs: i64,
        max_calls: u32,
        debug: bool,
    ) -> Self {
        let mut map = HashMap::new();
        let mut operations = Vec::new();
        for endpoint in endpoints {
            let fingerprint = credential_fingerprint(&endpoint.headers);
            operations.extend(endpoint.operations);
            map.insert(
                endpoint.adapter_id.clone(),
                Endpoint {
                    adapter_id: endpoint.adapter_id,
                    base_url: endpoint.base_url.trim_end_matches('/').to_string(),
                    headers: endpoint.headers,
                    fingerprint,
                },
            );
        }
        Self {
            endpoints: map,
            operations,
            http,
            cache,
            clock,
            ttl_secs,
            calls: AtomicU32::new(0),
            max_calls,
            debug,
        }
    }

    fn find(&self, operation_id: &str) -> Result<&Operation, InvokeError> {
        self.operations
            .iter()
            .find(|operation| operation.operation_id == operation_id)
            .ok_or_else(|| InvokeError::UnknownOperation(operation_id.to_string()))
    }

    fn endpoint_for(&self, operation_id: &str) -> Result<&Endpoint, InvokeError> {
        if let Some((adapter, _)) = operation_id.split_once("::") {
            if let Some(endpoint) = self.endpoints.get(adapter) {
                return Ok(endpoint);
            }
        }
        if self.endpoints.len() == 1 {
            return Ok(self
                .endpoints
                .values()
                .next()
                .expect("single endpoint is present"));
        }
        Err(InvokeError::UnknownOperation(operation_id.to_string()))
    }
}

impl OperationInvoker for HttpInvoker {
    fn operations(&self) -> &[Operation] {
        &self.operations
    }

    fn invoke(&self, operation_id: &str, args: &Value) -> Result<Value, InvokeError> {
        let call_number = self.calls.fetch_add(1, Ordering::SeqCst);
        if call_number >= self.max_calls {
            return Err(InvokeError::CallLimit(self.max_calls));
        }
        let operation = self.find(operation_id)?;
        let endpoint = self.endpoint_for(operation_id)?;
        let (url, body) = build_request(operation, &endpoint.base_url, args)?;
        let body_text = body
            .as_ref()
            .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".into()))
            .unwrap_or_default();
        let cacheable = operation.method == "GET" || operation.method == "HEAD";
        let key = cache_key(
            &endpoint.adapter_id,
            &operation.method,
            &url,
            &body_text,
            &endpoint.fingerprint,
        );
        if cacheable && self.ttl_secs > 0 {
            match self.cache.lookup_cached(&key, self.clock.unix_secs()) {
                Ok(Some(hit)) => {
                    if self.debug {
                        eprintln!(
                            "mcp-code-mode cache hit {} {}",
                            operation.method, operation.path
                        );
                    }
                    return parse_body(operation, hit.status, &hit.body);
                }
                Ok(None) => {}
                Err(error) => return Err(InvokeError::Cache(error.to_string())),
            }
        }
        if self.debug {
            eprintln!("mcp-code-mode {} {}", operation.method, operation.path);
        }
        let response = self
            .http
            .send(&HttpRequest {
                method: operation.method.clone(),
                url: url.clone(),
                headers: endpoint.headers.clone(),
                body: body.map(|_| body_text.clone()),
            })
            .map_err(|error| InvokeError::Transport {
                operation: operation.operation_id.clone(),
                message: error.to_string(),
            })?;
        if cacheable && (200..300).contains(&response.status) && self.ttl_secs > 0 {
            let cached = CachedHttp {
                adapter_id: endpoint.adapter_id.clone(),
                status: response.status,
                body: response.body.clone(),
            };
            self.cache
                .store_cached(&key, &cached, self.clock.unix_secs(), self.ttl_secs)
                .map_err(|error| InvokeError::Cache(error.to_string()))?;
        }
        parse_body(operation, response.status, &response.body)
    }
}

fn parse_body(operation: &Operation, status: u16, body: &str) -> Result<Value, InvokeError> {
    if !(200..300).contains(&status) {
        return Err(InvokeError::Http {
            operation: operation.operation_id.clone(),
            method: operation.method.clone(),
            path: operation.path.clone(),
            status,
            body: truncate(body, 2_000),
        });
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(body).map_err(|error| InvokeError::Transport {
        operation: operation.operation_id.clone(),
        message: format!("response was not JSON: {error}"),
    })
}

fn build_request(
    operation: &Operation,
    base_url: &str,
    args: &Value,
) -> Result<(String, Option<Value>), InvokeError> {
    let mut object = match args {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        _ => {
            return Err(InvokeError::InvalidArguments {
                operation: operation.operation_id.clone(),
                message: "arguments must be an object".into(),
            })
        }
    };
    let mut path = operation.path.clone();
    let mut query: Vec<(String, String)> = Vec::new();
    for parameter in &operation.parameters {
        match parameter.location {
            ParameterLocation::Path => {
                let value = object.remove(&parameter.name).ok_or_else(|| {
                    InvokeError::InvalidArguments {
                        operation: operation.operation_id.clone(),
                        message: format!("missing path parameter `{}`", parameter.name),
                    }
                })?;
                let text = scalar_string(&operation.operation_id, &parameter.name, &value)?;
                let needle = format!("{{{}}}", parameter.name);
                if !path.contains(&needle) {
                    return Err(InvokeError::InvalidArguments {
                        operation: operation.operation_id.clone(),
                        message: format!("path has no placeholder for `{}`", parameter.name),
                    });
                }
                path = path.replace(&needle, &encode_component(&text));
            }
            ParameterLocation::Query => {
                if let Some(value) = object.remove(&parameter.name) {
                    if !value.is_null() {
                        let text = scalar_string(&operation.operation_id, &parameter.name, &value)?;
                        query.push((parameter.name.clone(), text));
                    }
                } else if parameter.required {
                    return Err(InvokeError::InvalidArguments {
                        operation: operation.operation_id.clone(),
                        message: format!("missing query parameter `{}`", parameter.name),
                    });
                }
            }
            ParameterLocation::Header | ParameterLocation::Cookie => {}
        }
    }
    if path.contains('{') {
        return Err(InvokeError::InvalidArguments {
            operation: operation.operation_id.clone(),
            message: format!("unresolved path `{path}`"),
        });
    }
    let mut url = join_url(base_url, &path);
    if !query.is_empty() {
        let encoded = query
            .into_iter()
            .map(|(name, value)| {
                format!("{}={}", encode_component(&name), encode_component(&value))
            })
            .collect::<Vec<_>>()
            .join("&");
        url.push('?');
        url.push_str(&encoded);
    }
    let sends_body = matches!(operation.method.as_str(), "POST" | "PUT" | "PATCH");
    let body = if sends_body {
        Some(Value::Object(object))
    } else {
        None
    };
    Ok((url, body))
}

fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

fn scalar_string(operation: &str, name: &str, value: &Value) -> Result<String, InvokeError> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(flag) => Ok(flag.to_string()),
        _ => Err(InvokeError::InvalidArguments {
            operation: operation.to_string(),
            message: format!("parameter `{name}` must be a string, number, or bool"),
        }),
    }
}

fn encode_component(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn truncate(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpError, HttpResponse};
    use code_mode_core::{ManualClock, Parameter};
    use code_mode_index::SqliteStore;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct Scripted {
        responses: Mutex<VecDeque<HttpResponse>>,
        requests: Arc<Mutex<Vec<HttpRequest>>>,
    }

    impl HttpTransport for Scripted {
        fn send(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
            self.requests
                .lock()
                .expect("requests")
                .push(request.clone());
            self.responses
                .lock()
                .expect("responses")
                .pop_front()
                .ok_or_else(|| HttpError::Message("no scripted response".into()))
        }
    }

    fn operation(id: &str, method: &str, path: &str, params: Vec<Parameter>) -> Operation {
        Operation {
            operation_id: id.into(),
            binding_name: id.into(),
            method: method.into(),
            path: path.into(),
            summary: String::new(),
            description: String::new(),
            tags: Vec::new(),
            parameters: params,
            request_schema: json!({}),
            response_schema: json!({}),
        }
    }

    fn path_param(name: &str) -> Parameter {
        Parameter {
            name: name.into(),
            location: ParameterLocation::Path,
            required: true,
            description: String::new(),
            schema: json!({"type": "string"}),
        }
    }

    #[test]
    fn path_param_is_removed_from_the_body() {
        let op = operation(
            "update_page",
            "PATCH",
            "/v1/pages/{page_id}",
            vec![path_param("page_id")],
        );
        let (url, body) = build_request(
            &op,
            "https://api.notion.com/",
            &json!({"page_id": "abc/def", "properties": {"title": "Hi"}}),
        )
        .unwrap();
        assert_eq!(url, "https://api.notion.com/v1/pages/abc%2Fdef");
        let body = body.unwrap();
        assert!(body.get("page_id").is_none());
        assert_eq!(body["properties"]["title"], "Hi");
    }

    #[test]
    fn get_uses_ttl_cache_and_post_does_not() {
        let clock = Arc::new(ManualClock::new(1_000));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let transport = Scripted {
            responses: Mutex::new(VecDeque::from([
                HttpResponse {
                    status: 200,
                    body: "{\"id\":\"1\"}".into(),
                },
                HttpResponse {
                    status: 200,
                    body: "{\"id\":\"2\"}".into(),
                },
            ])),
            requests: Arc::clone(&requests),
        };
        let store = SqliteStore::open_in_memory().unwrap();
        let get_op = operation(
            "retrieve_page",
            "GET",
            "/v1/pages/{page_id}",
            vec![path_param("page_id")],
        );
        let invoker = HttpInvoker::new(
            InvokerParts {
                adapter_id: "notion".into(),
                base_url: "http://example.test".into(),
                headers: vec![("Authorization".into(), "Bearer secret".into())],
                operations: vec![get_op],
                ttl_secs: 30,
                max_calls: 8,
                debug: false,
            },
            Box::new(transport),
            store,
            clock.clone(),
        );
        let args = json!({"page_id": "abc"});
        let first = invoker.invoke("retrieve_page", &args).unwrap();
        let second = invoker.invoke("retrieve_page", &args).unwrap();
        assert_eq!(first, second);
        assert_eq!(requests.lock().unwrap().len(), 1);

        clock.set(1_031);
        let third = invoker.invoke("retrieve_page", &args).unwrap();
        assert_eq!(third["id"], "2");
        assert_eq!(requests.lock().unwrap().len(), 2);

        let post_requests = Arc::new(Mutex::new(Vec::new()));
        let post_transport = Scripted {
            responses: Mutex::new(VecDeque::from([
                HttpResponse {
                    status: 200,
                    body: "{\"id\":\"a\"}".into(),
                },
                HttpResponse {
                    status: 200,
                    body: "{\"id\":\"b\"}".into(),
                },
            ])),
            requests: Arc::clone(&post_requests),
        };
        let post = HttpInvoker::new(
            InvokerParts {
                adapter_id: "notion".into(),
                base_url: "http://example.test".into(),
                headers: vec![("Authorization".into(), "Bearer secret".into())],
                operations: vec![operation("create_page", "POST", "/v1/pages", Vec::new())],
                ttl_secs: 30,
                max_calls: 8,
                debug: false,
            },
            Box::new(post_transport),
            SqliteStore::open_in_memory().unwrap(),
            Arc::new(ManualClock::new(1_000)),
        );
        post.invoke("create_page", &json!({"title": "one"}))
            .unwrap();
        post.invoke("create_page", &json!({"title": "one"}))
            .unwrap();
        assert_eq!(post_requests.lock().unwrap().len(), 2);
        assert!(post_requests.lock().unwrap()[0]
            .headers
            .iter()
            .any(|(name, value)| name == "Authorization" && value == "Bearer secret"));
    }
}
