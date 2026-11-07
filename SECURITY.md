# Security policy

## Supported version

Security fixes target the latest version on the default branch.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Contact the maintainer privately through the contact options on [lemon.dev.br](https://lemon.dev.br) and include reproduction steps, impact, and affected versions. Avoid sending secrets or real provider tokens in reports. The maintainer will acknowledge the report and coordinate a fix and disclosure.

## Secret handling

Provider credentials belong in environment variables or the MCP client's protected environment configuration. Never commit tokens, paste them into Rhai scripts, or include them in diagnostics. Debug logging records request methods and paths, not authorization headers.
