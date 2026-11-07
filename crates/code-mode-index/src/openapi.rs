//! OpenAPI 3.0 / 3.1 JSON loader.
//!
//! The subset that matters for Code Mode: paths, methods, parameters,
//! request bodies, and JSON responses, with local `$ref` and simple `allOf`
//! inlining. Other APIs drop in by pointing the loader at their spec.

use crate::IndexError;
use code_mode_core::{Operation, Parameter, ParameterLocation};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

const METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];
const SCHEMA_REF: &str = "#/components/schemas/";
const PARAM_REF: &str = "#/components/parameters/";
const RESPONSE_REF: &str = "#/components/responses/";

struct Components {
    schemas: Map<String, Value>,
    parameters: Map<String, Value>,
    responses: Map<String, Value>,
    request_bodies: Map<String, Value>,
}

pub fn parse_openapi(spec_json: &str) -> Result<Vec<Operation>, IndexError> {
    let root: Value = serde_json::from_str(spec_json)
        .map_err(|error| IndexError::Spec(format!("document is not JSON: {error}")))?;
    let root = root
        .as_object()
        .ok_or_else(|| IndexError::Spec("document must be a JSON object".into()))?;
    let version = root
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or_else(|| IndexError::Spec("missing openapi version".into()))?;
    if !version.starts_with("3.") {
        return Err(IndexError::Spec(format!(
            "unsupported OpenAPI version `{version}` (need 3.x JSON)"
        )));
    }
    let components = read_components(root);
    let paths = root
        .get("paths")
        .and_then(Value::as_object)
        .ok_or_else(|| IndexError::Spec("missing paths object".into()))?;

    let mut operations = Vec::new();
    for (path, item) in paths {
        let item = item
            .as_object()
            .ok_or_else(|| IndexError::Spec(format!("path `{path}` is not an object")))?;
        if item.contains_key("$ref") {
            return Err(IndexError::Spec(format!(
                "path `{path}` uses $ref, which this loader does not inline"
            )));
        }
        let path_parameters = item
            .get("parameters")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        for method in METHODS {
            let Some(operation) = item.get(method) else {
                continue;
            };
            let operation = operation
                .as_object()
                .ok_or_else(|| IndexError::Spec(format!("`{method} {path}` is not an object")))?;
            operations.push(read_operation(
                method,
                path,
                operation,
                &path_parameters,
                &components,
            )?);
        }
    }
    assign_binding_names(&mut operations);
    operations.sort_by(|left, right| left.operation_id.cmp(&right.operation_id));
    Ok(operations)
}

fn read_components(root: &Map<String, Value>) -> Components {
    let empty = Map::new();
    let components = root
        .get("components")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    Components {
        schemas: object_map(components, "schemas"),
        parameters: object_map(components, "parameters"),
        responses: object_map(components, "responses"),
        request_bodies: object_map(components, "requestBodies"),
    }
}

fn object_map(parent: &Map<String, Value>, key: &str) -> Map<String, Value> {
    parent
        .get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn read_operation(
    method: &str,
    path: &str,
    operation: &Map<String, Value>,
    path_parameters: &Value,
    components: &Components,
) -> Result<Operation, IndexError> {
    let operation_id = operation
        .get("operationId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| synthesize_id(method, path));
    let mut stack = Vec::new();
    let mut parameters = Vec::new();
    let mut seen = BTreeSet::new();
    for source in [
        path_parameters,
        operation.get("parameters").unwrap_or(&Value::Null),
    ] {
        if source.is_null() {
            continue;
        }
        let list = source.as_array().ok_or_else(|| {
            IndexError::Spec(format!("parameters for `{operation_id}` must be an array"))
        })?;
        for parameter in list {
            let parsed = read_parameter(parameter, components, &mut stack)?;
            if seen.insert((parsed.name.clone(), location_key(parsed.location))) {
                parameters.push(parsed);
            }
        }
    }
    let request_schema = read_request_schema(operation, components, &mut stack)?;
    let response_schema = read_response_schema(operation, components, &mut stack)?;
    Ok(Operation {
        operation_id,
        binding_name: String::new(),
        method: method.to_ascii_uppercase(),
        path: path.to_string(),
        summary: text_field(operation, "summary"),
        description: text_field(operation, "description"),
        tags: string_array(operation.get("tags")),
        parameters,
        request_schema,
        response_schema,
    })
}

fn synthesize_id(method: &str, path: &str) -> String {
    format!("{method}_{path}")
}

fn text_field(operation: &Map<String, Value>, key: &str) -> String {
    operation
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn read_parameter(
    value: &Value,
    components: &Components,
    stack: &mut Vec<String>,
) -> Result<Parameter, IndexError> {
    let mut value = value.clone();
    if let Some(reference) = value.get("$ref").and_then(Value::as_str) {
        let name = reference
            .strip_prefix(PARAM_REF)
            .ok_or_else(|| IndexError::Spec(format!("unsupported parameter $ref `{reference}`")))?;
        value = components
            .parameters
            .get(name)
            .cloned()
            .ok_or_else(|| IndexError::Spec(format!("unknown parameter component `{name}`")))?;
    }
    let object = value
        .as_object()
        .ok_or_else(|| IndexError::Spec("parameter is not an object".into()))?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| IndexError::Spec("parameter is missing name".into()))?
        .to_string();
    let location = object
        .get("in")
        .and_then(Value::as_str)
        .ok_or_else(|| IndexError::Spec(format!("parameter `{name}` is missing in")))?;
    let location = match location {
        "path" => ParameterLocation::Path,
        "query" => ParameterLocation::Query,
        "header" => ParameterLocation::Header,
        "cookie" => ParameterLocation::Cookie,
        other => {
            return Err(IndexError::Spec(format!(
                "parameter `{name}` has unsupported location `{other}`"
            )))
        }
    };
    let schema = object
        .get("schema")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let schema = resolve(&schema, components, 0, stack)?;
    Ok(Parameter {
        name,
        location,
        required: object
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        description: object
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        schema,
    })
}

fn read_request_schema(
    operation: &Map<String, Value>,
    components: &Components,
    stack: &mut Vec<String>,
) -> Result<Value, IndexError> {
    let Some(body) = operation.get("requestBody") else {
        return Ok(Value::Null);
    };
    let body = dereference_request_body(body, components);
    schema_from_content(Some(&body), components, stack)
}

fn dereference_request_body(body: &Value, components: &Components) -> Value {
    let Some(reference) = body.get("$ref").and_then(Value::as_str) else {
        return body.clone();
    };
    let Some(name) = reference.strip_prefix("#/components/requestBodies/") else {
        return body.clone();
    };
    components
        .request_bodies
        .get(name)
        .cloned()
        .unwrap_or_else(|| body.clone())
}

fn read_response_schema(
    operation: &Map<String, Value>,
    components: &Components,
    stack: &mut Vec<String>,
) -> Result<Value, IndexError> {
    let Some(responses) = operation.get("responses").and_then(Value::as_object) else {
        return Ok(Value::Null);
    };
    let chosen = ["200", "201", "202", "204"]
        .into_iter()
        .find_map(|code| responses.get(code))
        .or_else(|| {
            responses
                .iter()
                .find(|(code, _)| code.starts_with('2'))
                .map(|(_, value)| value)
        });
    let Some(response) = chosen else {
        return Ok(Value::Null);
    };
    let response = if let Some(reference) = response.get("$ref").and_then(Value::as_str) {
        let name = reference
            .strip_prefix(RESPONSE_REF)
            .ok_or_else(|| IndexError::Spec(format!("unsupported response $ref `{reference}`")))?;
        components
            .responses
            .get(name)
            .cloned()
            .ok_or_else(|| IndexError::Spec(format!("unknown response component `{name}`")))?
    } else {
        response.clone()
    };
    schema_from_content(Some(&response), components, stack)
}

fn schema_from_content(
    container: Option<&Value>,
    components: &Components,
    stack: &mut Vec<String>,
) -> Result<Value, IndexError> {
    let Some(container) = container else {
        return Ok(Value::Null);
    };
    // Walk the map. A JSON pointer would treat the slash in `application/json` as a segment.
    let content = container
        .get("content")
        .and_then(|content| content.get("application/json"))
        .and_then(|media| media.get("schema"));
    match content {
        Some(schema) => resolve(schema, components, 0, stack),
        None => Ok(Value::Null),
    }
}

fn resolve(
    value: &Value,
    components: &Components,
    depth: usize,
    stack: &mut Vec<String>,
) -> Result<Value, IndexError> {
    if depth > 32 {
        return Ok(serde_json::json!({
            "truncated": true,
            "reason": "schema depth limit"
        }));
    }
    match value {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve(item, components, depth + 1, stack)?);
            }
            Ok(Value::Array(out))
        }
        Value::Object(map) => resolve_object(map, components, depth, stack),
        other => Ok(other.clone()),
    }
}

fn resolve_object(
    map: &Map<String, Value>,
    components: &Components,
    depth: usize,
    stack: &mut Vec<String>,
) -> Result<Value, IndexError> {
    if let Some(reference) = map.get("$ref").and_then(Value::as_str) {
        return resolve_schema_ref(reference, components, depth, stack);
    }
    if map.contains_key("allOf") {
        if let Some(merged) = try_merge_all_of(map, components, depth, stack)? {
            return Ok(merged);
        }
    }
    let mut out = Map::new();
    for (key, value) in map {
        out.insert(key.clone(), resolve(value, components, depth + 1, stack)?);
    }
    Ok(Value::Object(out))
}

fn resolve_schema_ref(
    reference: &str,
    components: &Components,
    depth: usize,
    stack: &mut Vec<String>,
) -> Result<Value, IndexError> {
    let Some(name) = reference.strip_prefix(SCHEMA_REF) else {
        // Examples, callbacks, and external refs stay as pointers. Official
        // specs use them beside the schemas we inline.
        return Ok(serde_json::json!({"$ref": reference}));
    };
    if stack.iter().any(|seen| seen == name) {
        return Ok(serde_json::json!({"$ref": reference, "circular": true}));
    }
    let schema = components
        .schemas
        .get(name)
        .cloned()
        .ok_or_else(|| IndexError::Spec(format!("unknown schema component `{name}`")))?;
    stack.push(name.to_string());
    let resolved = resolve(&schema, components, depth + 1, stack)?;
    stack.pop();
    Ok(resolved)
}

fn try_merge_all_of(
    map: &Map<String, Value>,
    components: &Components,
    depth: usize,
    stack: &mut Vec<String>,
) -> Result<Option<Value>, IndexError> {
    let Some(parts) = map.get("allOf").and_then(Value::as_array) else {
        return Ok(None);
    };
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut description = None;
    for part in parts {
        let resolved = resolve(part, components, depth + 1, stack)?;
        let Some(object) = resolved.as_object() else {
            return Ok(None);
        };
        if object.contains_key("allOf")
            || object.contains_key("oneOf")
            || object.contains_key("anyOf")
        {
            return Ok(None);
        }
        let is_object = object.get("properties").is_some()
            || object.get("required").is_some()
            || object.get("type").and_then(Value::as_str) == Some("object")
            || object.len() == 1 && object.contains_key("description");
        if !is_object && !object.is_empty() {
            return Ok(None);
        }
        if let Some(part_properties) = object.get("properties").and_then(Value::as_object) {
            for (key, value) in part_properties {
                properties.insert(key.clone(), value.clone());
            }
        }
        if let Some(part_required) = object.get("required").and_then(Value::as_array) {
            for item in part_required {
                if let Some(text) = item.as_str() {
                    if !required.iter().any(|existing| existing == text) {
                        required.push(text.to_string());
                    }
                }
            }
        }
        if description.is_none() {
            description = object
                .get("description")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
    }
    let mut merged = Map::new();
    merged.insert("type".into(), Value::String("object".into()));
    merged.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        merged.insert(
            "required".into(),
            Value::Array(required.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(description) = description {
        merged.insert("description".into(), Value::String(description));
    }
    for (key, value) in map {
        if key == "allOf" {
            continue;
        }
        let resolved = resolve(value, components, depth + 1, stack)?;
        if key == "properties" {
            if let (Some(base), Some(extra)) = (
                merged.get_mut("properties").and_then(Value::as_object_mut),
                resolved.as_object(),
            ) {
                for (name, schema) in extra {
                    base.insert(name.clone(), schema.clone());
                }
            }
        } else if key == "required" {
            // already merged from allOf; overlay extra required names
            if let Some(extra) = resolved.as_array() {
                let entry = merged
                    .entry(String::from("required"))
                    .or_insert_with(|| Value::Array(Vec::new()));
                if let Some(list) = entry.as_array_mut() {
                    for item in extra {
                        if list.iter().all(|existing| existing != item) {
                            list.push(item.clone());
                        }
                    }
                }
            }
        } else {
            merged.insert(key.clone(), resolved);
        }
    }
    Ok(Some(Value::Object(merged)))
}

fn assign_binding_names(operations: &mut [Operation]) {
    let mut used = BTreeSet::new();
    for operation in operations.iter_mut() {
        let mut name = sanitize_binding(&operation.operation_id);
        if name == "call" {
            name = "call_op".into();
        }
        if !used.insert(name.clone()) {
            let mut suffix = 2;
            loop {
                let candidate = format!("{name}_{suffix}");
                if used.insert(candidate.clone()) {
                    name = candidate;
                    break;
                }
                suffix += 1;
            }
        }
        operation.binding_name = name;
    }
}

fn sanitize_binding(operation_id: &str) -> String {
    let mut out = String::new();
    let mut underscore = false;
    for ch in operation_id.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            underscore = false;
        } else if !underscore {
            out.push('_');
            underscore = true;
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    let name = if trimmed.is_empty() {
        "operation".to_string()
    } else {
        trimmed
    };
    if name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("op_{name}")
    } else {
        name
    }
}

fn location_key(location: ParameterLocation) -> &'static str {
    match location {
        ParameterLocation::Path => "path",
        ParameterLocation::Query => "query",
        ParameterLocation::Header => "header",
        ParameterLocation::Cookie => "cookie",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_refs_all_of_and_path_parameters() {
        let spec = r##"{
            "openapi": "3.0.3",
            "info": {"title": "Example", "version": "1.0.0"},
            "paths": {
                "/v1/pages/{page_id}": {
                    "parameters": [
                        {"$ref": "#/components/parameters/PageId"}
                    ],
                    "patch": {
                        "operationId": "update-page",
                        "summary": "Update a page",
                        "tags": ["Pages"],
                        "requestBody": {
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/PageUpdate"}
                                }
                            }
                        },
                        "responses": {
                            "200": {"$ref": "#/components/responses/PageResponse"}
                        }
                    }
                }
            },
            "components": {
                "parameters": {
                    "PageId": {
                        "name": "page_id",
                        "in": "path",
                        "required": true,
                        "schema": {"type": "string"}
                    }
                },
                "schemas": {
                    "PageUpdate": {
                        "allOf": [
                            {"$ref": "#/components/schemas/WithProperties"}
                        ]
                    },
                    "WithProperties": {
                        "type": "object",
                        "required": ["properties"],
                        "properties": {"properties": {"type": "object"}}
                    },
                    "Page": {
                        "type": "object",
                        "properties": {"id": {"type": "string"}}
                    },
                    "Loop": {"$ref": "#/components/schemas/Loop"}
                },
                "responses": {
                    "PageResponse": {
                        "description": "ok",
                        "content": {
                            "application/json": {
                                "schema": {"$ref": "#/components/schemas/Page"}
                            }
                        }
                    }
                }
            }
        }"##;
        let ops = parse_openapi(spec).unwrap();
        assert_eq!(ops.len(), 1);
        let op = &ops[0];
        assert_eq!(op.operation_id, "update-page");
        assert_eq!(op.binding_name, "update_page");
        assert_eq!(op.method, "PATCH");
        assert_eq!(op.parameters.len(), 1);
        assert_eq!(op.parameters[0].name, "page_id");
        assert!(op.request_schema.get("$ref").is_none());
        assert!(op.request_schema["properties"]["properties"].is_object());
        assert_eq!(op.response_schema["properties"]["id"]["type"], "string");
    }

    #[test]
    fn circular_ref_does_not_recurse_forever() {
        let spec = r##"{
            "openapi": "3.1.0",
            "info": {"title": "Loop", "version": "0"},
            "paths": {
                "/loop": {
                    "get": {
                        "operationId": "loop",
                        "responses": {
                            "200": {
                                "description": "ok",
                                "content": {
                                    "application/json": {
                                        "schema": {"$ref": "#/components/schemas/Loop"}
                                    }
                                }
                            }
                        }
                    }
                }
            },
            "components": {
                "schemas": {"Loop": {"$ref": "#/components/schemas/Loop"}}
            }
        }"##;
        let ops = parse_openapi(spec).unwrap();
        assert_eq!(ops[0].response_schema["circular"], true);
    }

    #[test]
    fn duplicate_binding_names_are_suffixed() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "Dup", "version": "0"},
            "paths": {
                "/a": {"get": {"operationId": "do-thing"}},
                "/b": {"post": {"operationId": "do_thing"}}
            }
        }"#;
        let ops = parse_openapi(spec).unwrap();
        let names: Vec<_> = ops.iter().map(|op| op.binding_name.as_str()).collect();
        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1]);
    }
}
