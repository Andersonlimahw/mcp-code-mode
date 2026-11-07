use mcp_code_mode::{
    CodeModeApp, McpServer, NotionAdapter, SqliteStore, SystemClock, ToolProvider, UreqTransport,
    VERSION,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SCRIPT: &str = include_str!("../../../fixtures/workflow/workflow.rhai");
const FINAL: &str = include_str!("../../../fixtures/workflow/final-result.json");

fn body(contents: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(contents.to_string(), "application/json")
}

async fn mount(server: &MockServer, verb: &str, url_path: &str, contents: &str) {
    Mock::given(method(verb))
        .and(path(url_path))
        .respond_with(body(contents))
        .mount(server)
        .await;
}

fn app(base_url: &str) -> CodeModeApp {
    let adapter = NotionAdapter::new("test-token", base_url, "2022-06-28").unwrap();
    CodeModeApp::new(
        &adapter,
        SqliteStore::open_in_memory().unwrap(),
        Box::new(UreqTransport::new(Duration::from_secs(5))),
        Arc::new(SystemClock),
        60,
        16,
        false,
    )
    .unwrap()
}

#[tokio::test]
async fn five_step_workflow_returns_only_the_final_value() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/v1/search",
        include_str!("../../../fixtures/workflow/search-result.json"),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v1/pages",
        include_str!("../../../fixtures/workflow/create-page-result.json"),
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v1/pages/11111111-2222-4333-8444-555555555555",
        include_str!("../../../fixtures/workflow/update-page-result.json"),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v1/databases",
        include_str!("../../../fixtures/workflow/create-database-result.json"),
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v1/databases/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        include_str!("../../../fixtures/workflow/update-database-result.json"),
    )
    .await;

    let app = app(&server.uri());
    let result = app.call_tool("execute", json!({"code": SCRIPT})).unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(
        !result.content.contains("INTERMEDIATE_"),
        "intermediate bodies leaked into the tool result: {}",
        result.content
    );
    let actual: Value = serde_json::from_str(&result.content).unwrap();
    let expected: Value = serde_json::from_str(FINAL).unwrap();
    assert_eq!(actual, expected);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 5, "{requests:?}");
    let paths: Vec<_> = requests
        .iter()
        .map(|request| request.url.path().to_string())
        .collect();
    assert_eq!(
        paths,
        vec![
            "/v1/search",
            "/v1/pages",
            "/v1/pages/11111111-2222-4333-8444-555555555555",
            "/v1/databases",
            "/v1/databases/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        ]
    );
    for request in &requests {
        assert_eq!(
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer test-token")
        );
        assert_eq!(
            request
                .headers
                .get("notion-version")
                .and_then(|value| value.to_str().ok()),
            Some("2022-06-28")
        );
    }
    let update_page: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(update_page.get("page_id").is_none());
    assert_eq!(
        update_page["properties"]["title"]["title"][0]["text"]["content"],
        "Launch checklist (edited)"
    );
    let update_db: Value = serde_json::from_slice(&requests[4].body).unwrap();
    assert!(update_db.get("database_id").is_none());
    assert_eq!(update_db["title"][0]["text"]["content"], "Launch tasks v2");
}

#[tokio::test]
async fn update_block_and_append_children_hit_the_block_routes() {
    let server = MockServer::start().await;
    mount(
        &server,
        "PATCH",
        "/v1/blocks/99999999-8888-4777-8666-555555555555",
        r#"{"object":"block","id":"99999999-8888-4777-8666-555555555555","type":"paragraph"}"#,
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v1/blocks/11111111-2222-4333-8444-555555555555/children",
        r#"{"object":"list","results":[{"object":"block","id":"abababab-abab-4bab-8bab-abababababab","type":"paragraph"}]}"#,
    )
    .await;
    let app = app(&server.uri());
    let code = r#"
        let updated = update_block(#{
            block_id: "99999999-8888-4777-8666-555555555555",
            paragraph: #{
                rich_text: [#{ type: "text", text: #{ content: "Edited paragraph" } }]
            }
        });
        let appended = append_block_children(#{
            block_id: "11111111-2222-4333-8444-555555555555",
            children: [#{
                object: "block",
                type: "paragraph",
                paragraph: #{
                    rich_text: [#{ type: "text", text: #{ content: "New line" } }]
                }
            }]
        });
        #{ updated: updated.id, appended: appended.results[0].id }
    "#;
    let result = app.call_tool("execute", json!({"code": code})).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["updated"], "99999999-8888-4777-8666-555555555555");
    assert_eq!(value["appended"], "abababab-abab-4bab-8bab-abababababab");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let block: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(block.get("block_id").is_none());
    assert_eq!(
        block["paragraph"]["rich_text"][0]["text"]["content"],
        "Edited paragraph"
    );
}

#[tokio::test]
async fn mcp_execute_matches_the_direct_tool_call() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/v1/search",
        include_str!("../../../fixtures/workflow/search-result.json"),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v1/pages",
        include_str!("../../../fixtures/workflow/create-page-result.json"),
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v1/pages/11111111-2222-4333-8444-555555555555",
        include_str!("../../../fixtures/workflow/update-page-result.json"),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v1/databases",
        include_str!("../../../fixtures/workflow/create-database-result.json"),
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v1/databases/aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
        include_str!("../../../fixtures/workflow/update-database-result.json"),
    )
    .await;
    let app = app(&server.uri());
    let mcp = McpServer::new(app, "mcp-code-mode", VERSION);
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {"name": "execute", "arguments": {"code": SCRIPT}}
    });
    let response = mcp.handle_message(&request).unwrap();
    assert_eq!(response["result"]["isError"], false);
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let actual: Value = serde_json::from_str(text).unwrap();
    let expected: Value = serde_json::from_str(FINAL).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn stdio_initialize_and_tools_list() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};

    let mut child = Command::new(env!("CARGO_BIN_EXE_mcp-code-mode"))
        .env("NOTION_TOKEN", "stdio-token")
        .env("CODE_MODE_CACHE", ":memory:")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-03-26","capabilities":{{}},"clientInfo":{{"name":"test","version":"0"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    drop(stdin);
    let mut lines = BufReader::new(stdout).lines();
    let init: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let tools: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "mcp-code-mode");
    let names: Vec<_> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["list_apis", "search", "describe", "execute"]);
    let status = child.wait().unwrap();
    assert!(status.success());
}
