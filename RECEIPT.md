# Token receipt

Tokenizer: `tiktoken-rs` 0.6.0 function `o200k_base()`, which loads the **o200k_base** byte-pair encoding (the GPT-4o / o-series encoding). Counts are `CoreBPE::encode_with_special_tokens(text).len()`. `encode_ordinary` returned the same length for every measured string, so none of them contained a special token. These are not Claude tokenizer counts, and this file does not estimate those. No row is an estimate. Ratios are arithmetic on the measured integers and are labeled where they are rounded.

## Sources

- Official server: `@notionhq/notion-mcp-server` version `2.6.1`. Captured on 2026-11-07 by running the published `bin/cli.mjs` over stdio, sending `initialize` (protocol `2024-11-05`) and `tools/list`. `serverInfo` was `Notion API` version `1.0.0`. The fixture `fixtures/notion-mcp-2.6.1-tools.raw.json` is the exact `result.tools` array bytes from that response, not a re-serialization.
- This server: `mcp-code-mode` 0.1.0. Tools are `list_apis`, `search`, `describe`, and `execute`. The array is `serde_json::to_string` of `result.tools`.
- Workflow fixtures live in `fixtures/workflow/`. Result bodies are mock JSON shaped like the public API, not captures from a live Notion workspace. The counts below are the token counts of those fixtures.

## (a) Tool-definition tokens

What was counted: the JSON array of tool definitions a client receives from `tools/list` (`result.tools`). Commas and brackets are included. Per-tool rows below count each tool object only.

| Set | Tools | Bytes | o200k_base tokens |
| --- | ---: | ---: | ---: |
| Official Notion MCP `result.tools` | 24 | 76215 | 17500 |
| mcp-code-mode before (2 tools, previously measured) | 2 | 915 | 201 |
| mcp-code-mode after (`list_apis`, `search`, `describe`, `execute`) | 4 | 1647 | 355 |
| Difference (Notion official − ours after), computed | 20 | 74568 | 17145 |
| Ours / official, computed | | | 355/17500 = 0.0203 (computed from measured token counts, rounded to 4 decimal places) |

### Official tools

| Tool | Bytes | o200k_base tokens |
| --- | ---: | ---: |
| `API-get-user` | 2587 | 604 |
| `API-get-users` | 2885 | 663 |
| `API-get-self` | 2558 | 595 |
| `API-post-search` | 4805 | 1097 |
| `API-get-block-children` | 3028 | 693 |
| `API-patch-block-children` | 3083 | 713 |
| `API-retrieve-a-block` | 2644 | 612 |
| `API-update-a-block` | 3108 | 727 |
| `API-delete-a-block` | 2641 | 611 |
| `API-retrieve-a-page` | 2961 | 680 |
| `API-patch-page` | 3892 | 889 |
| `API-post-page` | 3566 | 819 |
| `API-retrieve-a-page-property` | 3071 | 704 |
| `API-retrieve-a-comment` | 3010 | 687 |
| `API-create-a-comment` | 3166 | 725 |
| `API-query-data-source` | 3317 | 770 |
| `API-retrieve-a-data-source` | 2680 | 618 |
| `API-update-a-data-source` | 3100 | 721 |
| `API-create-a-data-source` | 2953 | 688 |
| `API-list-data-source-templates` | 2775 | 640 |
| `API-retrieve-a-database` | 2662 | 613 |
| `API-move-page` | 2730 | 641 |
| `API-retrieve-page-markdown` | 3106 | 706 |
| `API-update-page-markdown` | 5862 | 1282 |

### mcp-code-mode tools

| Tool | Bytes | o200k_base tokens |
| --- | ---: | ---: |
| `list_apis` | 185 | 42 |
| `search` | 581 | 125 |
| `describe` | 455 | 92 |
| `execute` | 421 | 94 |

## (b) Five-step workflow

The workflow is search, create page, edit the page, create a database, edit the database.

Classic side: five `tools/call` rounds against `@notionhq/notion-mcp-server` 2.6.1. That server has no `POST /v1/databases` tool. Its create-database and update-database tools are `API-create-a-data-source` and `API-update-a-data-source` (Notion API 2025-09-03 data sources). Steps 4 and 5 use those tools and data-source shaped mock results. Steps 1–3 use `API-post-search`, `API-post-page`, and `API-patch-page` with the same page/search mock bodies as this server's pilot.

Code Mode side: one `execute` call whose script is `fixtures/workflow/workflow.rhai` (trimmed). The script calls `search`, `create_page`, `update_page`, `create_database`, and `update_database` on the 2022-06-28 pilot. The tool result is only the final JSON value. Intermediate bodies do not re-enter the count.

A classic call string is the compact JSON `{"name","arguments"}`. A classic result string is the compact JSON of the mock body (`serde_json::to_string` of the parsed fixture). The execute call string is the same shape, with the script in `arguments.code`, so JSON escaping is included. Tool-definition tokens from section (a) are not added again here.

| Step | Call bytes | Call tokens | Result bytes | Result tokens | Step tokens |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 search `API-post-search` | 123 | 30 | 628 | 229 | 259 |
| 2 create page `API-post-page` | 187 | 72 | 564 | 200 | 272 |
| 3 edit page `API-patch-page` | 186 | 55 | 589 | 208 | 263 |
| 4 create database `API-create-a-data-source` | 204 | 60 | 496 | 156 | 216 |
| 5 edit database `API-update-a-data-source` | 241 | 64 | 617 | 190 | 254 |
| **Classic total, computed** | 941 | 281 | 2894 | 983 | 1264 |

| Code Mode piece | Bytes | o200k_base tokens |
| --- | ---: | ---: |
| Script source, trimmed, before JSON escaping | 1392 | 332 |
| `execute` call (`name` + `arguments.code`) | 1528 | 426 |
| Final result only | 237 | 95 |
| **Single execute round-trip, computed** (call + final result) | 1765 | 521 |

### Comparison

| | Bytes | o200k_base tokens |
| --- | ---: | ---: |
| Classic 5 calls + 5 results | 3835 | 1264 |
| Single execute call + final result | 1765 | 521 |
| Classic − execute, computed | 2070 | 743 |
| Execute / classic, computed | | 521/1264 = 0.4122 (computed from measured token counts, rounded to 4 decimal places) |

### Not part of the single execute call

An agent usually calls `search` before writing the script. This measurement is that extra round-trip for query `page database` with limit 5. It is not included in the execute total above. Operation ids returned, in order: create_database, create_page, update_database, update_page, search.

| Piece | Bytes | o200k_base tokens |
| --- | ---: | ---: |
| `search` call | 65 | 16 |
| `search` result text | 901 | 215 |
| Search round-trip, computed | 966 | 231 |

### Search before and after

Same query `page database`, limit 5, Notion pilot. Before is the previous receipt (full schemas on every hit), labeled as previously measured and not re-counted. After is this run's compact `search` result.

| | Bytes | o200k_base tokens |
| --- | ---: | ---: |
| Before: search call (previously measured) | 65 | 16 |
| Before: search result (previously measured) | 8539 | 1963 |
| Before: round-trip (previously measured) | 8604 | 1979 |
| After: search call | 65 | 16 |
| After: search result | 901 | 215 |
| After: round-trip | 966 | 231 |
| Before − after round-trip, computed | 7638 | 1748 |

`describe` for `create_page` is separate: call 13 tokens, result 311 tokens, round-trip 324 tokens. `list_apis` round-trip is 35 tokens. Neither is part of the default search.

## Official MCP servers

Our column is the same four-tool `result.tools` array for every API. Official columns are captured `tools/list` payloads. Vercel has no captured payload.

| API | Official server | Version | Official tools | Official tokens | Our tokens |
| --- | --- | --- | ---: | ---: | ---: |
| Notion | `@notionhq/notion-mcp-server` | 2.6.1 | 24 | 17500 | 355 |
| GitHub | `github/github-mcp-server` default `stdio` | 2.1.0 | 46 | 57436 | 355 |
| Supabase | `@supabase/mcp-server-supabase` | 0.16.1 | 36 | 8036 | 355 |
| Vercel | hosted `https://mcp.vercel.com` (OAuth) | UNMEASURED | UNMEASURED | UNMEASURED | 355 |

Vercel tool-definition tokens are **UNMEASURED**. The official server is the hosted OAuth endpoint, and this run did not capture a `tools/list` body. Third-party npm servers were not substituted.

## Resident context, computed

These sums add measured rows from (a) and (b). They are arithmetic, not a new measurement and not an estimate.

| Context | o200k_base tokens |
| --- | ---: |
| Official tool definitions + 5 classic calls and results | 17500 + 1264 = 18764 |
| Our tool definitions + the search round-trip + the execute round-trip | 355 + 231 + 521 = 1107 |
| Classic resident − code-mode resident | 18764 − 1107 = 17657 |

## Audit hashes

SHA-256 of the exact strings that were tokenized.

| String | SHA-256 |
| --- | --- |
| Official `result.tools` bytes | `f9544e7c84986529d5bc4a1c9357bbbb9c2f458133904075319f8fec7d716da9` |
| mcp-code-mode `result.tools` bytes | `e5026853a22b1973126a08f3374ef74e17eb0b745cd4966be1d0193e1779ea4e` |
| 1 search call | `a0f562ca340c825920ad42fb6589984e077e89424a86bb7390247f634eff217a` |
| 1 search result | `67cdb9d2449c447554c1ed3e2908ba6313af473dbebe89b32afc295b239237c7` |
| 2 create page call | `22885682e3fc4441ee0019a6832d138d4b18193bbfdde9415f3883fb8e5bf119` |
| 2 create page result | `c7d3ee5a8bb920cf8f7670f9376769c6ebdfa8bafb105221a5ff4ef80a3973bc` |
| 3 edit page call | `bc7658a67561244008a75fd4f2544b1540b99be305d3f56537f0093a55144b99` |
| 3 edit page result | `70f248118b96b72121101928510a5aa6ee8b00e0121ae2c1078388a7201018e1` |
| 4 create database call | `47e2398d3d87e35963cf8c0e7528417f2ccdfb7f8b1e5defe4cb69f05b5f0a92` |
| 4 create database result | `67e2c80e57e19455fb5ede3fadde7c0c1f302adf74ce9e8685e8dd82231b9e7a` |
| 5 edit database call | `6a7cc8ece0a42a9cf27e65dacb6c48feb38842ebf7910fc91f4bf92e2ac7c746` |
| 5 edit database result | `07c8d5bb89c851d558483fb2f3caf58e9f5fe399e16475f0d47192c2f0033a7c` |
| Trimmed script | `2474247c25db8ce7ff565a39916564d6186a0232216247fb9a0567cbd6f82a1c` |
| Execute call | `17190bef680afa42882d0334a2478dc487060707059417e17ce70fcbea016f0a` |
| Final result | `c89f530e572bd1449582836385b3ba8d2066fbb0cf078bae8b09ca18666b6dc1` |
| Search call | `96c013e9f961165a10a9c303ab1b940bd2d5afe8d178a88822b791b8b4404dfd` |
| Search result | `6b3b3d19d6d6573994866e1575f0cd32537c3cce910092751a770023f0a2a7df` |

