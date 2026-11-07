//! Measured token receipt.
//!
//! Every count comes from `tiktoken-rs`'s `o200k_base` encoding. Nothing in
//! the generated markdown is an estimate; ratios are labeled as arithmetic on
//! those counts.

use crate::{
    tool_definitions, CodeModeApp, HttpTransport, McpServer, NotionAdapter, SqliteStore,
    SystemClock, ToolProvider, VERSION,
};
use code_mode_core::ToolDefinition;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tiktoken_rs::CoreBPE;

const OFFICIAL_TOOLS: &str = include_str!("../../../fixtures/notion-mcp-2.6.1-tools.raw.json");
const OFFICIAL_META: &str = include_str!("../../../fixtures/notion-mcp-2.6.1-meta.json");
const GITHUB_TOOLS: &str = include_str!("../../../fixtures/github-mcp-2.1.0-tools.raw.json");
const GITHUB_META: &str = include_str!("../../../fixtures/github-mcp-2.1.0-meta.json");
const SUPABASE_TOOLS: &str = include_str!("../../../fixtures/supabase-mcp-0.16.1-tools.raw.json");
const SUPABASE_META: &str = include_str!("../../../fixtures/supabase-mcp-0.16.1-meta.json");

/// Previously measured on 2026-11-07. Full schemas were inlined in every search
/// hit, and the server exposed only `search` and `execute`. Not re-counted.
const BEFORE_OUR_TOOL_BYTES: usize = 915;
const BEFORE_OUR_TOOL_TOKENS: usize = 201;
const BEFORE_SEARCH_CALL_BYTES: usize = 65;
const BEFORE_SEARCH_CALL_TOKENS: usize = 16;
const BEFORE_SEARCH_RESULT_BYTES: usize = 8539;
const BEFORE_SEARCH_RESULT_TOKENS: usize = 1963;
const BEFORE_SEARCH_ROUND_BYTES: usize = 8604;
const BEFORE_SEARCH_ROUND_TOKENS: usize = 1979;
const CARGO_LOCK: &str = include_str!("../../../Cargo.lock");
const SCRIPT: &str = include_str!("../../../fixtures/workflow/workflow.rhai");

const SEARCH_QUERY: &str = "page database";
const SEARCH_LIMIT: u64 = 5;

struct Counted {
    bytes: usize,
    tokens: usize,
    sha256: String,
}

pub fn render_receipt() -> String {
    let bpe = tiktoken_rs::o200k_base().expect("o200k_base encoding loads");
    let meta: Value = serde_json::from_str(OFFICIAL_META).expect("official meta is json");
    let official_elements = split_json_objects(OFFICIAL_TOOLS);
    assert_eq!(
        official_elements.len(),
        24,
        "official tools fixture should contain 24 objects, found {}",
        official_elements.len()
    );
    let official_array = count(&bpe, OFFICIAL_TOOLS);
    let our_array_json = our_tools_array_json();
    let our_elements = split_json_objects(&our_array_json);
    assert_eq!(
        our_elements.len(),
        4,
        "expected list_apis, search, describe, execute"
    );
    let our_array = count(&bpe, &our_array_json);
    let github_tools = count(&bpe, GITHUB_TOOLS);
    let github_meta: Value = serde_json::from_str(GITHUB_META).expect("github meta");
    let supabase_tools = count(&bpe, SUPABASE_TOOLS);
    let supabase_meta: Value = serde_json::from_str(SUPABASE_META).expect("supabase meta");
    assert_eq!(split_json_objects(GITHUB_TOOLS).len(), 46);
    assert_eq!(split_json_objects(SUPABASE_TOOLS).len(), 36);

    let mut ordinary_mismatch = false;
    for text in [OFFICIAL_TOOLS, our_array_json.as_str()] {
        if !ordinary_matches(&bpe, text) {
            ordinary_mismatch = true;
        }
    }

    let steps = classic_steps();
    let mut step_rows = Vec::new();
    let mut classic_call_tokens = 0;
    let mut classic_result_tokens = 0;
    let mut classic_call_bytes = 0;
    let mut classic_result_bytes = 0;
    for step in &steps {
        let call = count(&bpe, &step.call_json);
        let result = count(&bpe, &step.result_json);
        if !ordinary_matches(&bpe, &step.call_json) || !ordinary_matches(&bpe, &step.result_json) {
            ordinary_mismatch = true;
        }
        classic_call_tokens += call.tokens;
        classic_result_tokens += result.tokens;
        classic_call_bytes += call.bytes;
        classic_result_bytes += result.bytes;
        step_rows.push((step, call, result));
    }
    let classic_total_tokens = classic_call_tokens + classic_result_tokens;
    let classic_total_bytes = classic_call_bytes + classic_result_bytes;

    let script = SCRIPT.trim();
    let script_count = count(&bpe, script);
    let execute_call = serde_json::to_string(&json!({
        "name": "execute",
        "arguments": {"code": script}
    }))
    .expect("execute call serializes");
    let execute_call_count = count(&bpe, &execute_call);
    let final_result: Value =
        serde_json::from_str(include_str!("../../../fixtures/workflow/final-result.json"))
            .expect("final result json");
    let final_json = serde_json::to_string(&final_result).expect("final result serializes");
    let final_count = count(&bpe, &final_json);
    if !ordinary_matches(&bpe, script)
        || !ordinary_matches(&bpe, &execute_call)
        || !ordinary_matches(&bpe, &final_json)
    {
        ordinary_mismatch = true;
    }
    let execute_total_tokens = execute_call_count.tokens + final_count.tokens;
    let execute_total_bytes = execute_call_count.bytes + final_count.bytes;

    let search = measure_search(&bpe);
    if !ordinary_matches(&bpe, &search.call_json) || !ordinary_matches(&bpe, &search.result_json) {
        ordinary_mismatch = true;
    }

    let tokenizer_note = if ordinary_mismatch {
        "`encode_with_special_tokens` and `encode_ordinary` disagreed on at least one string, so a special token was present. The table uses `encode_with_special_tokens`."
    } else {
        "`encode_ordinary` returned the same length for every measured string, so none of them contained a special token."
    };

    let mut out = String::new();
    out.push_str("# Token receipt\n\n");
    out.push_str("Tokenizer: `tiktoken-rs` ");
    out.push_str(&package_version(CARGO_LOCK, "tiktoken-rs"));
    out.push_str(" function `o200k_base()`, which loads the **o200k_base** byte-pair encoding (the GPT-4o / o-series encoding). ");
    out.push_str("Counts are `CoreBPE::encode_with_special_tokens(text).len()`. ");
    out.push_str(tokenizer_note);
    out.push_str(" These are not Claude tokenizer counts, and this file does not estimate those. ");
    out.push_str("No row is an estimate. Ratios are arithmetic on the measured integers and are labeled where they are rounded.\n\n");

    out.push_str("## Sources\n\n");
    out.push_str(&format!(
        "- Official server: `{}` version `{}`. Captured on {} by running the published `{}` over stdio, sending `initialize` (protocol `{}`) and `tools/list`. `serverInfo` was `{}` version `{}`. The fixture `fixtures/notion-mcp-2.6.1-tools.raw.json` is the exact `result.tools` array bytes from that response, not a re-serialization.\n",
        meta["package"].as_str().unwrap_or("?"),
        meta["version"].as_str().unwrap_or("?"),
        meta["capturedOn"].as_str().unwrap_or("?"),
        meta["bin"].as_str().unwrap_or("?"),
        meta["protocolVersion"].as_str().unwrap_or("?"),
        meta["serverInfo"]["name"].as_str().unwrap_or("?"),
        meta["serverInfo"]["version"].as_str().unwrap_or("?")
    ));
    out.push_str(&format!(
        "- This server: `mcp-code-mode` {VERSION}. Tools are `list_apis`, `search`, `describe`, and `execute`. The array is `serde_json::to_string` of `result.tools`.\n"
    ));
    out.push_str("- Workflow fixtures live in `fixtures/workflow/`. Result bodies are mock JSON shaped like the public API, not captures from a live Notion workspace. The counts below are the token counts of those fixtures.\n\n");

    out.push_str("## (a) Tool-definition tokens\n\n");
    out.push_str("What was counted: the JSON array of tool definitions a client receives from `tools/list` (`result.tools`). Commas and brackets are included. Per-tool rows below count each tool object only.\n\n");
    out.push_str("| Set | Tools | Bytes | o200k_base tokens |\n");
    out.push_str("| --- | ---: | ---: | ---: |\n");
    out.push_str(&format!(
        "| Official Notion MCP `result.tools` | 24 | {} | {} |\n",
        official_array.bytes, official_array.tokens
    ));
    out.push_str(&format!(
        "| mcp-code-mode before (2 tools, previously measured) | 2 | {BEFORE_OUR_TOOL_BYTES} | {BEFORE_OUR_TOOL_TOKENS} |\n"
    ));
    out.push_str(&format!(
        "| mcp-code-mode after (`list_apis`, `search`, `describe`, `execute`) | 4 | {} | {} |\n",
        our_array.bytes, our_array.tokens
    ));
    out.push_str(&format!(
        "| Difference (Notion official − ours after), computed | {} | {} | {} |\n",
        24usize.saturating_sub(4),
        official_array.bytes - our_array.bytes,
        official_array.tokens - our_array.tokens
    ));
    out.push_str(&format!(
        "| Ours / official, computed | | | {} |\n\n",
        ratio(our_array.tokens, official_array.tokens)
    ));

    out.push_str("### Official tools\n\n");
    out.push_str("| Tool | Bytes | o200k_base tokens |\n| --- | ---: | ---: |\n");
    for element in &official_elements {
        let value: Value = serde_json::from_str(element).expect("tool object");
        let name = value["name"].as_str().unwrap_or("?");
        let counted = count(&bpe, element);
        out.push_str(&format!(
            "| `{name}` | {} | {} |\n",
            counted.bytes, counted.tokens
        ));
    }
    out.push_str("\n### mcp-code-mode tools\n\n");
    out.push_str("| Tool | Bytes | o200k_base tokens |\n| --- | ---: | ---: |\n");
    for element in &our_elements {
        let value: Value = serde_json::from_str(element).expect("tool object");
        let name = value["name"].as_str().unwrap_or("?");
        let counted = count(&bpe, element);
        out.push_str(&format!(
            "| `{name}` | {} | {} |\n",
            counted.bytes, counted.tokens
        ));
    }

    out.push_str("\n## (b) Five-step workflow\n\n");
    out.push_str("The workflow is search, create page, edit the page, create a database, edit the database.\n\n");
    out.push_str(
        "Classic side: five `tools/call` rounds against `@notionhq/notion-mcp-server` 2.6.1. ",
    );
    out.push_str("That server has no `POST /v1/databases` tool. Its create-database and update-database tools are `API-create-a-data-source` and `API-update-a-data-source` (Notion API 2025-09-03 data sources). ");
    out.push_str("Steps 4 and 5 use those tools and data-source shaped mock results. Steps 1–3 use `API-post-search`, `API-post-page`, and `API-patch-page` with the same page/search mock bodies as this server's pilot.\n\n");
    out.push_str("Code Mode side: one `execute` call whose script is `fixtures/workflow/workflow.rhai` (trimmed). The script calls `search`, `create_page`, `update_page`, `create_database`, and `update_database` on the 2022-06-28 pilot. The tool result is only the final JSON value. Intermediate bodies do not re-enter the count.\n\n");
    out.push_str("A classic call string is the compact JSON `{\"name\",\"arguments\"}`. A classic result string is the compact JSON of the mock body (`serde_json::to_string` of the parsed fixture). ");
    out.push_str("The execute call string is the same shape, with the script in `arguments.code`, so JSON escaping is included. ");
    out.push_str("Tool-definition tokens from section (a) are not added again here.\n\n");

    out.push_str(
        "| Step | Call bytes | Call tokens | Result bytes | Result tokens | Step tokens |\n",
    );
    out.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for (step, call, result) in &step_rows {
        out.push_str(&format!(
            "| {} `{}` | {} | {} | {} | {} | {} |\n",
            step.label,
            step.tool,
            call.bytes,
            call.tokens,
            result.bytes,
            result.tokens,
            call.tokens + result.tokens
        ));
    }
    out.push_str(&format!(
        "| **Classic total, computed** | {classic_call_bytes} | {classic_call_tokens} | {classic_result_bytes} | {classic_result_tokens} | {classic_total_tokens} |\n\n"
    ));

    out.push_str("| Code Mode piece | Bytes | o200k_base tokens |\n| --- | ---: | ---: |\n");
    out.push_str(&format!(
        "| Script source, trimmed, before JSON escaping | {} | {} |\n",
        script_count.bytes, script_count.tokens
    ));
    out.push_str(&format!(
        "| `execute` call (`name` + `arguments.code`) | {} | {} |\n",
        execute_call_count.bytes, execute_call_count.tokens
    ));
    out.push_str(&format!(
        "| Final result only | {} | {} |\n",
        final_count.bytes, final_count.tokens
    ));
    out.push_str(&format!(
        "| **Single execute round-trip, computed** (call + final result) | {execute_total_bytes} | {execute_total_tokens} |\n\n"
    ));

    out.push_str("### Comparison\n\n");
    out.push_str("| | Bytes | o200k_base tokens |\n| --- | ---: | ---: |\n");
    out.push_str(&format!(
        "| Classic 5 calls + 5 results | {classic_total_bytes} | {classic_total_tokens} |\n"
    ));
    out.push_str(&format!(
        "| Single execute call + final result | {execute_total_bytes} | {execute_total_tokens} |\n"
    ));
    out.push_str(&format!(
        "| Classic − execute, computed | {} | {} |\n",
        classic_total_bytes - execute_total_bytes,
        classic_total_tokens - execute_total_tokens
    ));
    out.push_str(&format!(
        "| Execute / classic, computed | | {} |\n\n",
        ratio(execute_total_tokens, classic_total_tokens)
    ));

    out.push_str("### Not part of the single execute call\n\n");
    out.push_str(&format!(
        "An agent usually calls `search` before writing the script. This measurement is that extra round-trip for query `{SEARCH_QUERY}` with limit {SEARCH_LIMIT}. It is not included in the execute total above. Operation ids returned, in order: {}.\n\n",
        search.operation_ids.join(", ")
    ));
    out.push_str("| Piece | Bytes | o200k_base tokens |\n| --- | ---: | ---: |\n");
    out.push_str(&format!(
        "| `search` call | {} | {} |\n",
        search.call.bytes, search.call.tokens
    ));
    out.push_str(&format!(
        "| `search` result text | {} | {} |\n",
        search.result.bytes, search.result.tokens
    ));
    let search_round_tokens = search.call.tokens + search.result.tokens;
    out.push_str(&format!(
        "| Search round-trip, computed | {} | {} |\n\n",
        search.call.bytes + search.result.bytes,
        search_round_tokens
    ));
    assert!(
        search_round_tokens < 300,
        "compact search round-trip should be under 300 tokens, got {search_round_tokens}"
    );

    out.push_str("### Search before and after\n\n");
    out.push_str("Same query `page database`, limit 5, Notion pilot. Before is the previous receipt (full schemas on every hit), labeled as previously measured and not re-counted. After is this run's compact `search` result.\n\n");
    out.push_str("| | Bytes | o200k_base tokens |\n| --- | ---: | ---: |\n");
    out.push_str(&format!(
        "| Before: search call (previously measured) | {BEFORE_SEARCH_CALL_BYTES} | {BEFORE_SEARCH_CALL_TOKENS} |\n"
    ));
    out.push_str(&format!(
        "| Before: search result (previously measured) | {BEFORE_SEARCH_RESULT_BYTES} | {BEFORE_SEARCH_RESULT_TOKENS} |\n"
    ));
    out.push_str(&format!(
        "| Before: round-trip (previously measured) | {BEFORE_SEARCH_ROUND_BYTES} | {BEFORE_SEARCH_ROUND_TOKENS} |\n"
    ));
    out.push_str(&format!(
        "| After: search call | {} | {} |\n",
        search.call.bytes, search.call.tokens
    ));
    out.push_str(&format!(
        "| After: search result | {} | {} |\n",
        search.result.bytes, search.result.tokens
    ));
    out.push_str(&format!(
        "| After: round-trip | {} | {search_round_tokens} |\n",
        search.call.bytes + search.result.bytes
    ));
    out.push_str(&format!(
        "| Before − after round-trip, computed | {} | {} |\n\n",
        BEFORE_SEARCH_ROUND_BYTES - (search.call.bytes + search.result.bytes),
        BEFORE_SEARCH_ROUND_TOKENS - search_round_tokens
    ));
    out.push_str(&format!(
        "`describe` for `create_page` is separate: call {} tokens, result {} tokens, round-trip {} tokens. `list_apis` round-trip is {} tokens. Neither is part of the default search.\n\n",
        search.describe_call.tokens,
        search.describe_result.tokens,
        search.describe_call.tokens + search.describe_result.tokens,
        search.list_call.tokens + search.list_result.tokens
    ));

    out.push_str("## Official MCP servers\n\n");
    out.push_str("Our column is the same four-tool `result.tools` array for every API. Official columns are captured `tools/list` payloads. Vercel has no captured payload.\n\n");
    out.push_str("| API | Official server | Version | Official tools | Official tokens | Our tokens |\n| --- | --- | --- | ---: | ---: | ---: |\n");
    out.push_str(&format!(
        "| Notion | `@notionhq/notion-mcp-server` | {} | 24 | {} | {} |\n",
        meta["version"].as_str().unwrap_or("?"),
        official_array.tokens,
        our_array.tokens
    ));
    out.push_str(&format!(
        "| GitHub | `github/github-mcp-server` default `stdio` | {} | {} | {} | {} |\n",
        github_meta["version"].as_str().unwrap_or("?"),
        github_meta["toolCount"].as_u64().unwrap_or(0),
        github_tools.tokens,
        our_array.tokens
    ));
    out.push_str(&format!(
        "| Supabase | `@supabase/mcp-server-supabase` | {} | {} | {} | {} |\n",
        supabase_meta["version"].as_str().unwrap_or("?"),
        supabase_meta["toolCount"].as_u64().unwrap_or(0),
        supabase_tools.tokens,
        our_array.tokens
    ));
    out.push_str("| Vercel | hosted `https://mcp.vercel.com` (OAuth) | UNMEASURED | UNMEASURED | UNMEASURED | ");
    out.push_str(&our_array.tokens.to_string());
    out.push_str(" |\n\n");
    out.push_str("Vercel tool-definition tokens are **UNMEASURED**. The official server is the hosted OAuth endpoint, and this run did not capture a `tools/list` body. Third-party npm servers were not substituted.\n\n");

    let classic_resident = official_array.tokens + classic_total_tokens;
    let code_resident = our_array.tokens + search_round_tokens + execute_total_tokens;
    out.push_str("## Resident context, computed\n\n");
    out.push_str("These sums add measured rows from (a) and (b). They are arithmetic, not a new measurement and not an estimate.\n\n");
    out.push_str("| Context | o200k_base tokens |\n| --- | ---: |\n");
    out.push_str(&format!(
        "| Official tool definitions + 5 classic calls and results | {official} + {classic} = {classic_resident} |\n",
        official = official_array.tokens,
        classic = classic_total_tokens,
    ));
    out.push_str(&format!(
        "| Our tool definitions + the search round-trip + the execute round-trip | {ours} + {search_tokens} + {execute} = {code_resident} |\n",
        ours = our_array.tokens,
        search_tokens = search_round_tokens,
        execute = execute_total_tokens,
    ));
    out.push_str(&format!(
        "| Classic resident − code-mode resident | {classic_resident} − {code_resident} = {} |\n\n",
        classic_resident - code_resident
    ));

    out.push_str("## Audit hashes\n\n");
    out.push_str("SHA-256 of the exact strings that were tokenized.\n\n");
    out.push_str("| String | SHA-256 |\n| --- | --- |\n");
    out.push_str(&format!(
        "| Official `result.tools` bytes | `{}` |\n",
        official_array.sha256
    ));
    out.push_str(&format!(
        "| mcp-code-mode `result.tools` bytes | `{}` |\n",
        our_array.sha256
    ));
    for (step, call, result) in &step_rows {
        out.push_str(&format!(
            "| {} call | `{}` |\n| {} result | `{}` |\n",
            step.label, call.sha256, step.label, result.sha256
        ));
    }
    out.push_str(&format!("| Trimmed script | `{}` |\n", script_count.sha256));
    out.push_str(&format!(
        "| Execute call | `{}` |\n| Final result | `{}` |\n",
        execute_call_count.sha256, final_count.sha256
    ));
    out.push_str(&format!(
        "| Search call | `{}` |\n| Search result | `{}` |\n",
        search.call.sha256, search.result.sha256
    ));
    out.push('\n');
    out
}

struct StepMeasured {
    label: &'static str,
    tool: &'static str,
    call_json: String,
    result_json: String,
}

fn classic_steps() -> Vec<StepMeasured> {
    let specs = [
        (
            "1 search",
            "API-post-search",
            include_str!("../../../fixtures/workflow/classic-search-args.json"),
            include_str!("../../../fixtures/workflow/search-result.json"),
        ),
        (
            "2 create page",
            "API-post-page",
            include_str!("../../../fixtures/workflow/classic-create-page-args.json"),
            include_str!("../../../fixtures/workflow/create-page-result.json"),
        ),
        (
            "3 edit page",
            "API-patch-page",
            include_str!("../../../fixtures/workflow/classic-update-page-args.json"),
            include_str!("../../../fixtures/workflow/update-page-result.json"),
        ),
        (
            "4 create database",
            "API-create-a-data-source",
            include_str!("../../../fixtures/workflow/classic-create-datasource-args.json"),
            include_str!("../../../fixtures/workflow/classic-create-datasource-result.json"),
        ),
        (
            "5 edit database",
            "API-update-a-data-source",
            include_str!("../../../fixtures/workflow/classic-update-datasource-args.json"),
            include_str!("../../../fixtures/workflow/classic-update-datasource-result.json"),
        ),
    ];
    specs
        .into_iter()
        .map(|(label, tool, args, result)| {
            let args: Value = serde_json::from_str(args).expect(label);
            let result: Value = serde_json::from_str(result).expect(label);
            StepMeasured {
                label,
                tool,
                call_json: serde_json::to_string(&json!({"name": tool, "arguments": args}))
                    .expect("call"),
                result_json: serde_json::to_string(&result).expect("result"),
            }
        })
        .collect()
}

struct SearchMeasured {
    call_json: String,
    result_json: String,
    call: Counted,
    result: Counted,
    operation_ids: Vec<String>,
    describe_call: Counted,
    describe_result: Counted,
    list_call: Counted,
    list_result: Counted,
}

fn measure_search(bpe: &CoreBPE) -> SearchMeasured {
    let adapter = NotionAdapter::new("receipt-token", "https://api.notion.com", "2022-06-28")
        .expect("notion adapter");
    let app = CodeModeApp::new(
        &adapter,
        SqliteStore::open_in_memory().expect("memory db"),
        Box::new(DenyHttp),
        Arc::new(SystemClock),
        60,
        4,
        false,
    )
    .expect("app");
    let arguments = json!({"query": SEARCH_QUERY, "limit": SEARCH_LIMIT});
    let call_json =
        serde_json::to_string(&json!({"name": "search", "arguments": arguments.clone()}))
            .expect("search call");
    let tool = app.call_tool("search", arguments).expect("search tool");
    assert!(!tool.is_error, "{}", tool.content);
    let parsed: Value = serde_json::from_str(&tool.content).expect("search json");
    let operation_ids = parsed["matches"]
        .as_array()
        .expect("matches")
        .iter()
        .map(|hit| hit["operation_id"].as_str().unwrap_or("?").to_string())
        .collect();
    let result = count(bpe, &tool.content);
    let call = count(bpe, &call_json);
    let describe_args = json!({"operation_id": "create_page"});
    let describe_call_json =
        serde_json::to_string(&json!({"name": "describe", "arguments": describe_args.clone()}))
            .expect("describe call");
    let described = app
        .call_tool("describe", describe_args)
        .expect("describe tool");
    assert!(!described.is_error, "{}", described.content);
    let list_args = json!({});
    let list_call_json =
        serde_json::to_string(&json!({"name": "list_apis", "arguments": list_args.clone()}))
            .expect("list call");
    let listed = app.call_tool("list_apis", list_args).expect("list_apis");
    assert!(!listed.is_error, "{}", listed.content);
    SearchMeasured {
        call_json,
        result_json: tool.content,
        call,
        result,
        operation_ids,
        describe_call: count(bpe, &describe_call_json),
        describe_result: count(bpe, &described.content),
        list_call: count(bpe, &list_call_json),
        list_result: count(bpe, &listed.content),
    }
}

struct DenyHttp;

impl HttpTransport for DenyHttp {
    fn send(&self, _request: &crate::HttpRequest) -> Result<crate::HttpResponse, crate::HttpError> {
        Err(crate::HttpError::Message(
            "receipt search must not call the network".into(),
        ))
    }
}

fn our_tools_array_json() -> String {
    let server = McpServer::new(Defs, "mcp-code-mode", VERSION);
    let response = server
        .handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .expect("tools/list");
    serde_json::to_string(&response["result"]["tools"]).expect("tools array")
}

struct Defs;

impl ToolProvider for Defs {
    fn instructions(&self) -> &str {
        crate::INSTRUCTIONS
    }

    fn list_tools(&self) -> Vec<ToolDefinition> {
        tool_definitions()
    }

    fn call_tool(
        &self,
        name: &str,
        _arguments: Value,
    ) -> Result<code_mode_core::ToolResult, code_mode_transport::ToolCallError> {
        Err(code_mode_transport::ToolCallError::UnknownTool(name.into()))
    }
}

fn count(bpe: &CoreBPE, text: &str) -> Counted {
    Counted {
        bytes: text.len(),
        tokens: bpe.encode_with_special_tokens(text).len(),
        sha256: sha256_hex(text.as_bytes()),
    }
}

fn ordinary_matches(bpe: &CoreBPE, text: &str) -> bool {
    bpe.encode_ordinary(text).len() == bpe.encode_with_special_tokens(text).len()
}

fn ratio(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "undefined (whole is 0)".into();
    }
    let rounded = (part as f64) / (whole as f64);
    format!("{part}/{whole} = {rounded:.4} (computed from measured token counts, rounded to 4 decimal places)")
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn package_version(lock: &str, name: &str) -> String {
    let needle = format!("name = \"{name}\"");
    let Some(index) = lock.find(&needle) else {
        return "not-found".into();
    };
    for line in lock[index + needle.len()..].lines() {
        let line = line.trim();
        if let Some(version) = line.strip_prefix("version = \"") {
            if let Some(version) = version.strip_suffix('"') {
                return version.to_string();
            }
        }
        if line.starts_with("name = ") || line.starts_with("[[package]]") {
            break;
        }
    }
    "not-found".into()
}

fn split_json_objects(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let Some(start) = raw.find('[') else {
        return out;
    };
    let mut rest = &raw[start + 1..];
    loop {
        let trimmed = rest.trim_start();
        let offset = rest.len() - trimmed.len();
        rest = &rest[offset..];
        if rest.starts_with(']') || rest.is_empty() {
            break;
        }
        if rest.starts_with(',') {
            rest = &rest[1..];
            continue;
        }
        if !rest.starts_with('{') {
            break;
        }
        let mut depth = 0;
        let mut in_string = false;
        let mut escape = false;
        let mut end = None;
        for (index, ch) in rest.char_indices() {
            if in_string {
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' => in_string = true,
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(index + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            break;
        };
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitter_reads_the_official_tool_array() {
        assert_eq!(split_json_objects(OFFICIAL_TOOLS).len(), 24);
    }

    #[test]
    fn tiktoken_version_is_recorded() {
        let version = package_version(CARGO_LOCK, "tiktoken-rs");
        assert_ne!(version, "not-found");
    }

    #[test]
    fn committed_receipt_matches_renderer() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../RECEIPT.md");
        let on_disk = std::fs::read_to_string(&path).expect("RECEIPT.md is present");
        assert_eq!(on_disk, render_receipt());
    }
}
