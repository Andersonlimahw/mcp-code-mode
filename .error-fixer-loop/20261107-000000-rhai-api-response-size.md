# Fix prompt: Rhai API response size handling

## Role / Context

Work in the Rust workspace `mcp-code-mode`, a Code Mode MCP server with a Rhai sandbox. A pre-existing regression test sets `ScriptLimits.max_string_size` to 20 and expects API response fields larger than 20 characters to remain usable.

## Task

Keep script string limits separate from API response limits. Permit ordinary large provider responses while rejecting responses above a configurable finite byte cap.

## Inputs / files

- `crates/code-mode-sandbox/src/engine.rs`
- `AGENTS.md`
- `docs/rules/sandbox-response-limits.md`

## Constraints

Preserve unrelated edits and never print or write provider token values. Keep the response cap finite and enforce it before injecting API JSON into Rhai.

## Plan

1. Inspect existing sandbox limits and tests. Verify: identify separate script and response caps.
2. Add regression coverage for accepted and oversized API response payloads. Verify: focused sandbox tests pass.
3. Run workspace formatting, tests, and clippy. Verify: every requested gate has an explicit exit code.

## Output contract

Report changed files, root cause, verification commands, exit codes, and remaining limitations.

## Feedback — DO

Preserve caller limits and make response handling bounded and testable.

## Feedback — DO NOT

Do not remove tests, disable limits, include credentials, or claim a gate passed unless it ran successfully.

```text
EXEC-MAP v1
intent: fix-bug
executor: codex
effort: medium
time: ~10-30 min
tokens: ~8k-25k
skills: [error-fixer-loop]
models: {plan: quality, impl: balanced, mechanical: budget}
agents: inline
mcp: []
router: heuristic
router_mode: heuristic
router_confidence: unavailable
router_fallback: static-catalog
notes: Rust sandbox regression; keep response limits finite and independent.
```
