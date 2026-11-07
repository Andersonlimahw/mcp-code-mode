# Contributing

Thanks for helping improve mcp-code-mode. Keep changes focused, preserve unrelated work, and never include credentials or live tokens in source, fixtures, logs, or test output.

## Development

Install the Rust version pinned in `rust-toolchain.toml`. Then run:

```bash
cargo fmt
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Adapter changes should include contract tests using a mock HTTP server. Keep provider credentials out of tests; use synthetic values and local mock endpoints. Update vendored spec provenance in `specs/vendor/SOURCES.json` when refreshing upstream documents. Update `RECEIPT.md` only through the `token-receipt` generator when measured inputs change.

When changing sandbox limits, keep script limits independent from bounded API response limits and cover both with regression tests.

## Pull requests

Describe behavior, compatibility impact, and local validation. Include tests for bugs and new adapter behavior. Do not include generated secrets, unrelated formatting, or unverified live-service claims.
