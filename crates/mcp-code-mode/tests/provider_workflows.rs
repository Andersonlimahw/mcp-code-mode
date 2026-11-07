use mcp_code_mode::{
    CodeModeApp, GitHubAdapter, SqliteStore, SupabaseAdapter, SystemClock, ToolProvider,
    UreqTransport, VercelAdapter,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn app(adapter: &dyn mcp_code_mode::ApiAdapter) -> CodeModeApp {
    CodeModeApp::new(
        adapter,
        SqliteStore::open_in_memory().unwrap(),
        Box::new(UreqTransport::new(Duration::from_secs(5))),
        Arc::new(SystemClock),
        0,
        8,
        false,
    )
    .unwrap()
}

async fn mount(server: &MockServer, verb: &str, url_path: &str, body: &str) {
    Mock::given(method(verb))
        .and(path(url_path))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body.to_string(), "application/json"))
        .mount(server)
        .await;
}

#[tokio::test]
async fn github_three_step_workflow() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/user", r#"{"login":"octocat","id":1}"#).await;
    mount(
        &server,
        "POST",
        "/user/repos",
        r#"{"name":"smoke","owner":{"login":"octocat"}}"#,
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/repos/octocat/smoke",
        r#"{"name":"smoke","description":"edited"}"#,
    )
    .await;
    let adapter = GitHubAdapter::new("ghp_test", server.uri()).unwrap();
    let app = app(&adapter);
    let code = r#"
        let me = users_get_authenticated(#{});
        let repo = repos_create_for_authenticated_user(#{
            name: "smoke",
            description: "demo"
        });
        let updated = repos_update(#{
            owner: me.login,
            repo: repo.name,
            description: "edited"
        });
        #{ login: me.login, repo: updated.name }
    "#;
    let result = app.call_tool("execute", json!({"code": code})).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["login"], "octocat");
    assert_eq!(value["repo"], "smoke");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url.path(), "/user");
    assert_eq!(requests[1].url.path(), "/user/repos");
    assert_eq!(requests[2].url.path(), "/repos/octocat/smoke");
    for request in &requests {
        assert_eq!(
            request
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer ghp_test")
        );
        assert_eq!(
            request
                .headers
                .get("accept")
                .and_then(|value| value.to_str().ok()),
            Some("application/vnd.github+json")
        );
        assert_eq!(
            request
                .headers
                .get("x-github-api-version")
                .and_then(|value| value.to_str().ok()),
            Some("2022-11-28")
        );
    }
    let update: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(update.get("owner").is_none());
    assert!(update.get("repo").is_none());
    assert_eq!(update["description"], "edited");
}

#[tokio::test]
async fn vercel_three_step_workflow() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v10/projects",
        r#"{"projects":[{"id":"prj_existing"}]}"#,
    )
    .await;
    mount(
        &server,
        "POST",
        "/v11/projects",
        r#"{"id":"prj_new","name":"code-mode"}"#,
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v9/projects/prj_new",
        r#"{"id":"prj_new","name":"code-mode-edited"}"#,
    )
    .await;
    let adapter = VercelAdapter::new("vercel_test", server.uri()).unwrap();
    let app = app(&adapter);
    let code = r#"
        let listed = getProjects(#{});
        let created = createProject(#{ name: "code-mode" });
        let updated = updateProject(#{ idOrName: created.id, name: "code-mode-edited" });
        #{ listed: listed.projects[0].id, id: updated.id, name: updated.name }
    "#;
    let result = app.call_tool("execute", json!({"code": code})).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["listed"], "prj_existing");
    assert_eq!(value["id"], "prj_new");
    assert_eq!(value["name"], "code-mode-edited");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.url.path().to_string())
            .collect::<Vec<_>>(),
        vec!["/v10/projects", "/v11/projects", "/v9/projects/prj_new"]
    );
    assert_eq!(
        requests[2]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer vercel_test")
    );
    let body: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(body.get("idOrName").is_none());
    assert_eq!(body["name"], "code-mode-edited");
}

#[tokio::test]
async fn supabase_three_step_workflow() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/v1/organizations", r#"[{"id":"org_1"}]"#).await;
    mount(
        &server,
        "GET",
        "/v1/projects",
        r#"[{"id":"ref_1","name":"demo"}]"#,
    )
    .await;
    mount(
        &server,
        "PATCH",
        "/v1/projects/ref_1",
        r#"{"id":"ref_1","name":"renamed"}"#,
    )
    .await;
    let adapter = SupabaseAdapter::new("sbp_test", server.uri()).unwrap();
    let app = app(&adapter);
    let code = r#"
        let orgs = v1_list_all_organizations(#{});
        let projects = v1_list_all_projects(#{});
        let updated = v1_update_a_project(#{ ref: projects[0].id, name: "renamed" });
        #{ org: orgs[0].id, project: updated.id, name: updated.name }
    "#;
    let result = app.call_tool("execute", json!({"code": code})).unwrap();
    assert!(!result.is_error, "{}", result.content);
    let value: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(value["org"], "org_1");
    assert_eq!(value["project"], "ref_1");
    assert_eq!(value["name"], "renamed");
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].url.path(), "/v1/projects/ref_1");
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sbp_test")
    );
    let body: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(body.get("ref").is_none());
    assert_eq!(body["name"], "renamed");
}
