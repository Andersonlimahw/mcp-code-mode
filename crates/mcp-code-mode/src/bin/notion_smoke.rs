//! Live Notion smoke test.
//!
//! Gated on `NOTION_TOKEN` and `NOTION_PARENT_PAGE_ID`. When either is missing
//! the process exits before any HTTP call. When both are set it creates a page
//! under that parent, edits the page, creates a database under the page, edits
//! the database, and prints an o200k_base token receipt of the real responses.

use mcp_code_mode::{
    CodeModeApp, HttpError, HttpRequest, HttpResponse, HttpTransport, NotionAdapter, SqliteStore,
    SystemClock, ToolProvider, UreqTransport,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tiktoken_rs::CoreBPE;

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("notion-smoke: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> anyhow::Result<()> {
    let token = nonempty("NOTION_TOKEN");
    let parent = nonempty("NOTION_PARENT_PAGE_ID");
    if let Some(missing) = gate_reason(token.as_deref(), parent.as_deref()) {
        eprintln!("notion-smoke is gated. Set {missing}. No request was sent to Notion.");
        std::process::exit(2);
    }
    let token = token.expect("gate checked the token");
    let parent = parent.expect("gate checked the parent");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let title = format!("mcp-code-mode smoke {stamp}");
    let edited = format!("{title} (edited)");
    let database_title = format!("smoke db {stamp}");
    let database_edited = format!("smoke db {stamp} (edited)");

    let records = Arc::new(Mutex::new(Vec::new()));
    let transport = RecordingTransport {
        inner: UreqTransport::new(Duration::from_secs(30)),
        records: Arc::clone(&records),
    };
    let adapter = NotionAdapter::new(token, "https://api.notion.com", "2022-06-28")?;
    let app = CodeModeApp::new(
        &adapter,
        SqliteStore::open_in_memory()?,
        Box::new(transport),
        Arc::new(SystemClock),
        0,
        8,
        false,
    )?;
    let code = smoke_script(&parent, &title, &edited, &database_title, &database_edited);
    let result = app.call_tool("execute", json!({"code": code}))?;
    let bpe = tiktoken_rs::o200k_base()?;
    let snapshot: Vec<Recorded> = records.lock().expect("records").iter().cloned().collect();
    print_receipt(&bpe, &snapshot, &result.content, result.is_error);
    if result.is_error {
        anyhow::bail!("script failed");
    }
    Ok(())
}

fn nonempty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn gate_reason(token: Option<&str>, parent: Option<&str>) -> Option<&'static str> {
    match (present(token), present(parent)) {
        (false, false) => Some("NOTION_TOKEN and NOTION_PARENT_PAGE_ID"),
        (false, true) => Some("NOTION_TOKEN"),
        (true, false) => Some("NOTION_PARENT_PAGE_ID"),
        (true, true) => None,
    }
}

fn present(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty())
}

fn smoke_script(
    parent: &str,
    title: &str,
    edited: &str,
    database_title: &str,
    database_edited: &str,
) -> String {
    format!(
        r#"
let page = create_page(#{{
    parent: #{{ page_id: {parent} }},
    properties: #{{
        title: #{{
            title: [#{{ type: "text", text: #{{ content: {title} }} }}]
        }}
    }}
}});
let edited = update_page(#{{
    page_id: page.id,
    properties: #{{
        title: #{{
            title: [#{{ type: "text", text: #{{ content: {edited} }} }}]
        }}
    }}
}});
let database = create_database(#{{
    parent: #{{ type: "page_id", page_id: page.id }},
    title: [#{{ type: "text", text: #{{ content: {database_title} }} }}],
    properties: #{{ Name: #{{ title: #{{}} }} }}
}});
let revised = update_database(#{{
    database_id: database.id,
    title: [#{{ type: "text", text: #{{ content: {database_edited} }} }}]
}});
#{{ page_id: edited.id, database_id: revised.id }}
"#,
        parent = rhai_string(parent),
        title = rhai_string(title),
        edited = rhai_string(edited),
        database_title = rhai_string(database_title),
        database_edited = rhai_string(database_edited),
    )
}

fn rhai_string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

#[derive(Clone)]
struct Recorded {
    method: String,
    url: String,
    status: u16,
    body: String,
}

struct RecordingTransport {
    inner: UreqTransport,
    records: Arc<Mutex<Vec<Recorded>>>,
}

impl HttpTransport for RecordingTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, HttpError> {
        let response = self.inner.send(request)?;
        self.records.lock().expect("records").push(Recorded {
            method: request.method.clone(),
            url: request.url.clone(),
            status: response.status,
            body: response.body.clone(),
        });
        Ok(response)
    }
}

fn print_receipt(bpe: &CoreBPE, records: &[Recorded], final_text: &str, failed: bool) {
    println!("# Notion live smoke receipt");
    println!();
    println!(
        "Tokenizer: `tiktoken-rs` `o200k_base` via `encode_with_special_tokens`. Counts below are measured from this run. They are not estimates and they are not Claude tokens."
    );
    println!();
    if failed {
        println!("The script returned an error. Response rows are whatever Notion sent before the failure.");
        println!();
        println!("```");
        println!("{final_text}");
        println!("```");
        println!();
    }
    println!("| Step | Status | Bytes | o200k_base tokens |");
    println!("| --- | ---: | ---: | ---: |");
    let mut response_tokens = 0usize;
    for (index, record) in records.iter().enumerate() {
        let tokens = bpe.encode_with_special_tokens(&record.body).len();
        response_tokens += tokens;
        println!(
            "| {} {} {} | {} | {} | {} |",
            index + 1,
            record.method,
            record.url,
            record.status,
            record.body.len(),
            tokens
        );
    }
    let final_tokens = bpe.encode_with_special_tokens(final_text).len();
    println!(
        "| Final script result | | {} | {final_tokens} |",
        final_text.len()
    );
    println!();
    println!(
        "Response bodies total {response_tokens} tokens. Final result is {final_tokens} tokens. Difference (responses − final), computed: {}.",
        response_tokens.saturating_sub(final_tokens)
    );
    if !failed {
        println!();
        println!("Final value:");
        println!();
        println!("```");
        println!("{final_text}");
        println!("```");
    }
}

#[cfg(test)]
mod tests {
    use super::{gate_reason, rhai_string};

    #[test]
    fn gate_requires_both_env_vars() {
        assert_eq!(
            gate_reason(None, None),
            Some("NOTION_TOKEN and NOTION_PARENT_PAGE_ID")
        );
        assert_eq!(gate_reason(Some(""), Some("page")), Some("NOTION_TOKEN"));
        assert_eq!(
            gate_reason(Some("token"), None),
            Some("NOTION_PARENT_PAGE_ID")
        );
        assert_eq!(gate_reason(Some("token"), Some("page")), None);
    }

    #[test]
    fn rhai_string_escapes_quotes() {
        assert_eq!(rhai_string("a\"b"), r#""a\"b""#);
    }
}
