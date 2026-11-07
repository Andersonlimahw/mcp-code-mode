---
name: mcp-code-mode
description: Migrate an MCP server or HTTP API onto mcp-code-mode. One process loads many OpenAPI adapters and exposes four tools (list_apis, search, describe, execute) instead of one tool per operation. Use when replacing a large tools/list, adding Notion, GitHub, Vercel, Supabase, or another REST API, or measuring tool-definition tokens.
---

# mcp-code-mode

One binary loads one or more OpenAPI adapters. The model does not see a tool per endpoint. It sees four tools:

| Tool | Use it for |
| --- | --- |
| `list_apis` | Which APIs are loaded (id, base URL, operation count) |
| `search` | Compact signatures: operation id, binding, `METHOD /path`, one-line summary, required params as `name:in:type` |
| `describe` | Full parameters and request/response schemas for one operation |
| `execute` | One Rhai script that calls the bindings and returns only the final expression |

`search` stays small. Call `describe` only for the operation you are about to use. Intermediate HTTP bodies stay in the sandbox.

## Several APIs in one server

Tokens that turn an adapter on:

| Adapter | Token env | Default origin | Spec |
| --- | --- | --- | --- |
| `notion` | `NOTION_TOKEN` | `https://api.notion.com` | embedded `specs/notion-2022-06-28.json` (`Notion-Version: 2022-06-28`) |
| `github` | `GITHUB_TOKEN` | `https://api.github.com` | `specs/vendor/github-rest-openapi.json` |
| `vercel` | `VERCEL_TOKEN` | `https://api.vercel.com` | `specs/vendor/vercel-rest-openapi.json` |
| `supabase` | `SUPABASE_ACCESS_TOKEN` | `https://api.supabase.com` | `specs/vendor/supabase-management-openapi.json` |

Sources, `info.version`, and SHA-256 are in `specs/vendor/SOURCES.json`.

If `CODE_MODE_ADAPTERS` is unset, every adapter whose token is set is loaded. To pin the set:

```bash
export CODE_MODE_ADAPTERS=notion,github
export NOTION_TOKEN=ntn_...
export GITHUB_TOKEN=github_pat_...
mcp-code-mode
```

With one API, bindings are unprefixed (`create_page`). With more than one, they are `{adapter}_{operation}` (`notion_create_page`, `github_repos_update`). `search` shows the binding to call. Pass `"adapter": "github"` to limit a search. Pass `adapter` to `describe` when the same operation id exists on two APIs.

`CODE_MODE_ADAPTER_ID` still selects a single adapter, including a custom id that is not one of the four above. A custom id needs `CODE_MODE_SPEC` and `CODE_MODE_BASE_URL`. `CODE_MODE_BASE_URL` and `CODE_MODE_SPEC` apply only when exactly one adapter is selected.

## When to migrate

Migrate when an MCP server publishes one tool per endpoint and `tools/list` dominates the prompt. Keep a normal MCP tool only when the operation is not an HTTP call the script can make (local UI, long-lived subscriptions, a user approval prompt).

## Add an API

1. Obtain an OpenAPI 3.0 or 3.1 **JSON** document. The loader does not read YAML. Convert YAML first.
2. If the API is bearer-token JSON, do not add a tool per operation. Either vendor the file next to the other specs and add a thin adapter like `GitHubAdapter`, or start a single custom adapter:

```bash
export CODE_MODE_ADAPTER_ID=github
export CODE_MODE_BASE_URL=https://api.github.com
export CODE_MODE_SPEC=./specs/github.json
export CODE_MODE_TOKEN_ENV=GITHUB_TOKEN
export GITHUB_TOKEN=github_pat_...
export CODE_MODE_EXTRA_HEADERS="Accept:application/vnd.github+json;X-GitHub-Api-Version:2022-11-28"
mcp-code-mode
```

`CODE_MODE_EXTRA_HEADERS` is `Name:Value` pairs separated by `;`. It is applied only for a single selected adapter. Named adapters already set their own headers (GitHub's `Accept` and `X-GitHub-Api-Version`, Notion's `Notion-Version`).

3. If the API needs a signing scheme, multipart uploads, or a non-JSON body, implement `ApiAdapter` and an `HttpTransport`. Do not force that through `OpenApiAdapter`.
4. Operation ids become Rhai bindings. Hyphens and slashes become underscores (`repos/update` → `repos_update`, and `github_repos_update` when several APIs are loaded). Path and query parameters are keys on the argument map and are stripped before the JSON body is sent. `search` returns the binding. `describe` returns the schema.
5. Call `search` and confirm the operations you care about rank first. If summaries are thin, add `summary` and `description` in the spec. The index is SQLite FTS5 plus a lexical score over operation id, path, summary, description, and tags.
6. Write agent instructions the way `AGENTS.md` does: `list_apis` if needed, `search`, `describe` only when the schema is required, then one `execute` that returns a small final value.
7. Add a wiremock integration test next to `crates/mcp-code-mode/tests/provider_workflows.rs`. Assert the HTTP paths, that path parameters are absent from the body, and that the tool result does not contain intermediate payloads.

## Notion pilot

`NotionAdapter` is the pattern for a named API: id, default origin, version header, and an embedded spec. The pilot covers search, create page, update page, update block, append block children, create database, and update database on `Notion-Version: 2022-06-28`.

The official `@notionhq/notion-mcp-server` 2.6.1 speaks the 2025-09-03 data-source API. To follow that surface, select only the Notion adapter, set `CODE_MODE_SPEC` to that OpenAPI file, and set `CODE_MODE_NOTION_VERSION=2025-09-03`. The four MCP tools stay the same.

`cargo run --bin notion-smoke` is a live check. It refuses to run unless both `NOTION_TOKEN` and `NOTION_PARENT_PAGE_ID` are set. It then creates a page under that parent, edits the page, creates a database under the page, edits the database, and prints an `o200k_base` receipt of the real response bodies versus the final script value. `cargo test` does not run it.

## What not to do

- Do not register an MCP tool per operation. That recreates the token cost this server removes.
- Do not put full schemas back into `search`. That tool is the compact path. `describe` is the schema path.
- Do not return full API bodies from the script's final expression unless the user asked for them.
- Do not cache POST, PATCH, PUT, or DELETE. The SQLite TTL cache stores successful GET and HEAD responses only.
- Do not give the script a general HTTP client. Bindings are the indexed operations.

## Measure

`cargo run --bin token-receipt -- RECEIPT.md` writes measured `o200k_base` counts (`tiktoken-rs` 0.6.0):

- tool-definition tokens for this server's four tools, with the earlier two-tool count labeled as previously measured
- the compact `search` round-trip for `page database` versus the earlier full-schema search
- `describe` and `list_apis` as separate measured rows
- a 5-step Notion workflow of classic tool calls plus mock results versus one `execute` call plus its final value
- official `tools/list` payloads for Notion MCP 2.6.1, `github-mcp-server` 2.1.0, and `@supabase/mcp-server-supabase` 0.16.1

The official Vercel MCP is the hosted OAuth server at `https://mcp.vercel.com`. Its tool-definition tokens are **UNMEASURED**. Do not substitute a third-party server and do not invent a count.

Capture another server's real `tools/list` the way `fixtures/notion-mcp-2.6.1-tools.raw.json` was captured: run the published binary, send `initialize` and `tools/list`, store the exact `result.tools` bytes. Do not re-serialize before counting. Label mock result bodies as fixtures.

## Traits

| Trait | Implement when |
| --- | --- |
| `ApiAdapter` | You are adding an API (id, base URL, spec JSON, headers) |
| `HttpTransport` | You need a different HTTP stack or a test double |
| `OperationInvoker` | Call routing is not "method + path + JSON body" |
| `Clock` | Tests need to expire the TTL cache |
| `ToolProvider` | You are attaching `list_apis`, `search`, `describe`, and `execute` to the stdio server |
