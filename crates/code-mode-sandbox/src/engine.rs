use crate::convert::{dynamic_to_json, json_to_dynamic};
use code_mode_core::{OperationInvoker, SandboxError};
use rhai::{Dynamic, Engine, EvalAltResult, Map, AST};
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct ScriptLimits {
    pub max_operations: u64,
    pub max_call_levels: usize,
    pub max_expr_depth: usize,
    pub max_string_size: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
    pub max_script_chars: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_call_levels: 64,
            max_expr_depth: 64,
            max_string_size: 200_000,
            max_array_size: 5_000,
            max_map_size: 5_000,
            max_script_chars: 100_000,
        }
    }
}

pub fn execute(
    code: &str,
    invoker: Arc<dyn OperationInvoker>,
    limits: &ScriptLimits,
) -> Result<Value, SandboxError> {
    let code = code.trim();
    if code.is_empty() {
        return Err(SandboxError::Empty);
    }
    if code.chars().count() > limits.max_script_chars {
        return Err(SandboxError::TooLarge(limits.max_script_chars));
    }
    let mut engine = Engine::new();
    configure(&mut engine, limits);
    register_bindings(&mut engine, invoker);
    let ast = engine
        .compile(code)
        .map_err(|error| SandboxError::Script(error.to_string()))?;
    eval_ast(&engine, ast)
}

fn configure(engine: &mut Engine, limits: &ScriptLimits) {
    engine.set_max_operations(limits.max_operations);
    engine.set_max_call_levels(limits.max_call_levels);
    engine.set_max_expr_depths(limits.max_expr_depth, limits.max_expr_depth);
    engine.set_max_string_size(limits.max_string_size);
    engine.set_max_array_size(limits.max_array_size);
    engine.set_max_map_size(limits.max_map_size);
    engine.set_strict_variables(true);
    // print/debug would write onto the MCP stdout stream and break framing.
    engine.on_print(|_| {});
    engine.on_debug(|_, _, _| {});
    let _ = engine.disable_symbol("eval");
}

fn register_bindings(engine: &mut Engine, invoker: Arc<dyn OperationInvoker>) {
    let operations: Vec<(String, String)> = invoker
        .operations()
        .iter()
        .map(|operation| {
            (
                operation.binding_name.clone(),
                operation.operation_id.clone(),
            )
        })
        .collect();
    for (binding, operation_id) in operations {
        let invoker = Arc::clone(&invoker);
        engine.register_fn(
            &binding,
            move |args: Map| -> Result<Dynamic, Box<EvalAltResult>> {
                invoke_map(invoker.as_ref(), &operation_id, args)
            },
        );
    }
    let listed = invoker.clone();
    engine.register_fn(
        "call",
        move |name: &str, args: Map| -> Result<Dynamic, Box<EvalAltResult>> {
            let operation_id = listed
                .operations()
                .iter()
                .find(|operation| operation.operation_id == name || operation.binding_name == name)
                .map(|operation| operation.operation_id.clone())
                .ok_or_else(|| runtime_error(format!("unknown operation `{name}`")))?;
            invoke_map(listed.as_ref(), &operation_id, args)
        },
    );
}

fn invoke_map(
    invoker: &dyn OperationInvoker,
    operation_id: &str,
    args: Map,
) -> Result<Dynamic, Box<EvalAltResult>> {
    let json = dynamic_to_json(&Dynamic::from_map(args)).map_err(runtime_error)?;
    let value = invoker
        .invoke(operation_id, &json)
        .map_err(|error| runtime_error(error.to_string()))?;
    Ok(json_to_dynamic(&value))
}

fn eval_ast(engine: &Engine, ast: AST) -> Result<Value, SandboxError> {
    let dynamic = engine
        .eval_ast::<Dynamic>(&ast)
        .map_err(|error| SandboxError::Script(error.to_string()))?;
    dynamic_to_json(&dynamic).map_err(SandboxError::Script)
}

fn runtime_error(message: impl Into<String>) -> Box<EvalAltResult> {
    Box::new(EvalAltResult::ErrorRuntime(
        message.into().into(),
        rhai::Position::NONE,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use code_mode_core::{InvokeError, Operation, OperationInvoker};
    use serde_json::json;
    use std::sync::Mutex;

    struct Stub {
        operations: Vec<Operation>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl OperationInvoker for Stub {
        fn operations(&self) -> &[Operation] {
            &self.operations
        }

        fn invoke(&self, operation_id: &str, args: &Value) -> Result<Value, InvokeError> {
            self.calls
                .lock()
                .expect("lock")
                .push((operation_id.to_string(), args.clone()));
            match operation_id {
                "search" => Ok(json!({"results": [{"id": "parent-1"}]})),
                "create_page" => Ok(json!({"id": "page-1"})),
                "fail" => Err(InvokeError::Http {
                    operation: "fail".into(),
                    method: "POST".into(),
                    path: "/fail".into(),
                    status: 400,
                    body: "bad".into(),
                }),
                _ => Err(InvokeError::UnknownOperation(operation_id.into())),
            }
        }
    }

    fn operation(id: &str) -> Operation {
        Operation {
            operation_id: id.into(),
            binding_name: id.into(),
            method: "POST".into(),
            path: format!("/{id}"),
            summary: String::new(),
            description: String::new(),
            tags: Vec::new(),
            parameters: Vec::new(),
            request_schema: json!({}),
            response_schema: json!({}),
        }
    }

    #[test]
    fn returns_only_the_final_expression() {
        let stub = Arc::new(Stub {
            operations: vec![operation("search"), operation("create_page")],
            calls: Mutex::new(Vec::new()),
        });
        let value = execute(
            r#"
            let found = search(#{ query: "Launch" });
            let page = create_page(#{ parent: #{ page_id: found.results[0].id } });
            #{ page_id: page.id, hits: found.results.len() }
            "#,
            stub.clone(),
            &ScriptLimits::default(),
        )
        .unwrap();
        assert_eq!(value["page_id"], "page-1");
        assert_eq!(value["hits"], 1);
        let calls = stub.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1["query"], "Launch");
        assert!(value.get("results").is_none());
    }

    #[test]
    fn http_error_becomes_a_script_error() {
        let stub = Arc::new(Stub {
            operations: vec![operation("fail")],
            calls: Mutex::new(Vec::new()),
        });
        let error = execute("fail(#{})", stub, &ScriptLimits::default()).unwrap_err();
        assert!(error.to_string().contains("400"), "{error}");
    }

    #[test]
    fn operation_limit_stops_a_loop() {
        let stub = Arc::new(Stub {
            operations: Vec::new(),
            calls: Mutex::new(Vec::new()),
        });
        let limits = ScriptLimits {
            max_operations: 50,
            ..ScriptLimits::default()
        };
        let error =
            execute("let i = 0; while i < 100000 { i += 1; } i", stub, &limits).unwrap_err();
        assert!(error.to_string().contains("operation"), "{error}");
    }

    #[test]
    fn eval_is_disabled() {
        let stub = Arc::new(Stub {
            operations: Vec::new(),
            calls: Mutex::new(Vec::new()),
        });
        let error = execute(r#"eval("1 + 1")"#, stub, &ScriptLimits::default()).unwrap_err();
        assert!(
            error.to_string().contains("eval") || error.to_string().contains("disabled"),
            "{error}"
        );
    }
}
