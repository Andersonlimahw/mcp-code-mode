# Sandbox response limits

- Keep API response caps independent from script string caps. Enforce serialized response bytes before converting values into Rhai, and verify both accepted and rejected boundary cases with `cargo test -p code-mode-sandbox`.
