# Using mcp-code-mode

This file describes the current MCP API and Rhai sandbox behavior. Keep examples and operation guidance aligned with the bundled Notion spec and provider OpenAPI documents. Do not put provider credentials in scripts, fixtures, logs, or documentation examples; use placeholders only.

Sandbox size-limit rule: [docs/rules/sandbox-response-limits.md](docs/rules/sandbox-response-limits.md).

API operations are not tools. They are functions inside `execute`. The tools are `list_apis`, `search`, `describe`, and `execute`.

## 1. See which APIs are loaded

`list_apis` returns ids such as `notion`, `github`, `vercel`, and `supabase`. With one API, bindings are unprefixed (`create_page`). With more than one, call `notion_create_page`, `github_repos_update`, and so on.

## 2. Search

Call `search` with a short query and a small limit.

```json
{ "query": "create page", "limit": 5 }
```

Each match has `operation_id`, `binding`, `http` (`POST /v1/pages`), `summary`, and `required`. A required entry is `name:in:type`, for example `parent:body:object` or `page_id:path:string`. Path and query names are keys of the argument map, not JSON body fields.

Pass `"adapter": "github"` to search one API. A query like `page database` covers the Notion workflow.

## 3. Describe one operation

Call `describe` with `operation_id` when you need the request or response schema. Do not describe every match. Pass `adapter` if the same id exists on two APIs.

## 4. Execute once

Put the whole workflow in one script. Chain calls. Keep intermediate payloads in local variables. The tool result is only the final expression, so end with the object the user actually needs.

```rhai
let found = search(#{ query: "Launch plan" });
let parent_id = found.results[0].id;
let page = create_page(#{
    parent: #{ page_id: parent_id },
    properties: #{
        title: #{
            title: [#{ type: "text", text: #{ content: "Launch checklist" } }]
        }
    }
});
let edited = update_page(#{
    page_id: page.id,
    properties: #{
        title: #{
            title: [#{ type: "text", text: #{ content: "Launch checklist (edited)" } }]
        }
    }
});
#{ page_id: edited.id }
```

Bindings for the Notion pilot:

| Binding | HTTP |
| --- | --- |
| `search(body)` | `POST /v1/search` |
| `create_page(body)` | `POST /v1/pages` |
| `update_page({ page_id, ...body })` | `PATCH /v1/pages/{page_id}` |
| `update_block({ block_id, ...body })` | `PATCH /v1/blocks/{block_id}` |
| `append_block_children({ block_id, ...body })` | `PATCH /v1/blocks/{block_id}/children` |
| `create_database(body)` | `POST /v1/databases` |
| `update_database({ database_id, ...body })` | `PATCH /v1/databases/{database_id}` |

`call("create_page", #{ ... })` is the same call by operation id or binding name.

Argument maps use `#{ key: value }`. Nested objects and arrays are JSON objects and arrays. A page id from a previous response is `page.id`.

## Rules

- Do not return a full page, database, or search payload unless the user asked for that payload. Pick ids, titles, and counts in the final expression.
- Do not split a workflow into several `execute` calls when one script can do it. Each call re-enters the model context.
- On an HTTP error, read the status and body in the tool error, fix the script, and call `execute` again.
- The sandbox has no filesystem and no network except the API bindings. `print` is discarded. `eval` is disabled.
- A script may make at most `CODE_MODE_MAX_CALLS` HTTP calls (default 32).

## Auth

The Notion adapter reads `NOTION_TOKEN` and sends `Authorization: Bearer …` plus `Notion-Version` (default `2022-06-28`). Do not put the token in the script.
