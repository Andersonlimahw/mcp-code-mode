use mcp_code_mode::{
    serve_stdio, ApiAdapter, CodeModeApp, GitHubAdapter, NotionAdapter, OpenApiAdapter,
    SqliteStore, SupabaseAdapter, SystemClock, UreqTransport, VercelAdapter, VERSION,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let app = match build_from_env() {
        Ok(app) => app,
        Err(error) => {
            eprintln!("mcp-code-mode: {error}");
            std::process::exit(1);
        }
    };
    eprintln!(
        "mcp-code-mode {VERSION} listening on stdio (apis: {})",
        app.adapter_ids().join(", ")
    );
    if let Err(error) = serve_stdio(app, "mcp-code-mode", VERSION) {
        eprintln!("mcp-code-mode: {error}");
        std::process::exit(1);
    }
}

fn build_from_env() -> anyhow::Result<CodeModeApp> {
    let selected = selected_adapters()?;
    let single = selected.len() == 1;
    let base_override = std::env::var("CODE_MODE_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let spec_override = std::env::var("CODE_MODE_SPEC")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let extra = parse_extra_headers(&std::env::var("CODE_MODE_EXTRA_HEADERS").unwrap_or_default())?;
    let mut adapters: Vec<Box<dyn ApiAdapter>> = Vec::new();
    for id in &selected {
        adapters.push(build_adapter(
            id,
            single,
            base_override.clone(),
            spec_override.clone(),
            extra.clone(),
        )?);
    }
    let refs: Vec<&dyn ApiAdapter> = adapters.iter().map(|adapter| adapter.as_ref()).collect();
    let store = open_cache()?;
    let timeout = env_u64("CODE_MODE_HTTP_TIMEOUT_SECS", 20)?;
    let ttl = env_i64("CODE_MODE_CACHE_TTL", 60)?;
    let max_calls = env_u32("CODE_MODE_MAX_CALLS", 32)?;
    let debug = std::env::var("CODE_MODE_DEBUG").ok().as_deref() == Some("1");
    Ok(CodeModeApp::from_adapters(
        &refs,
        store,
        Box::new(UreqTransport::new(Duration::from_secs(timeout))),
        Arc::new(SystemClock),
        ttl,
        max_calls,
        debug,
    )?)
}

fn selected_adapters() -> anyhow::Result<Vec<String>> {
    if let Some(list) = nonempty_var("CODE_MODE_ADAPTERS") {
        let ids: Vec<String> = list
            .split(',')
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        if ids.is_empty() {
            anyhow::bail!("CODE_MODE_ADAPTERS is empty");
        }
        return Ok(ids);
    }
    if let Some(id) = nonempty_var("CODE_MODE_ADAPTER_ID") {
        return Ok(vec![id]);
    }
    let mut ids = Vec::new();
    for (id, var) in [
        ("notion", "NOTION_TOKEN"),
        ("github", "GITHUB_TOKEN"),
        ("vercel", "VERCEL_TOKEN"),
        ("supabase", "SUPABASE_ACCESS_TOKEN"),
    ] {
        if nonempty_var(var).is_some() {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        anyhow::bail!(
            "set NOTION_TOKEN, GITHUB_TOKEN, VERCEL_TOKEN, or SUPABASE_ACCESS_TOKEN, or set CODE_MODE_ADAPTERS"
        );
    }
    Ok(ids)
}

fn build_adapter(
    id: &str,
    single: bool,
    base_override: Option<String>,
    spec_override: Option<String>,
    extra: Vec<(String, String)>,
) -> anyhow::Result<Box<dyn ApiAdapter>> {
    let base = |default: &str| {
        if single {
            base_override.clone().unwrap_or_else(|| default.to_string())
        } else {
            default.to_string()
        }
    };
    match id {
        "notion" => {
            let token = required_var("NOTION_TOKEN")?;
            let mut notion = NotionAdapter::new(
                token,
                base("https://api.notion.com"),
                env_or("CODE_MODE_NOTION_VERSION", "2022-06-28"),
            )?;
            if single {
                if let Some(path) = spec_override {
                    notion = notion.with_spec(std::fs::read_to_string(path)?);
                }
                for (name, value) in extra {
                    notion.push_header(name, value);
                }
            }
            Ok(Box::new(notion))
        }
        "github" => Ok(Box::new(GitHubAdapter::new(
            required_var("GITHUB_TOKEN")?,
            base("https://api.github.com"),
        )?)),
        "vercel" => Ok(Box::new(VercelAdapter::new(
            required_var("VERCEL_TOKEN")?,
            base("https://api.vercel.com"),
        )?)),
        "supabase" => Ok(Box::new(SupabaseAdapter::new(
            required_var("SUPABASE_ACCESS_TOKEN")?,
            base("https://api.supabase.com"),
        )?)),
        other => {
            if !single {
                anyhow::bail!("unknown adapter `{other}`");
            }
            let path = spec_override.ok_or_else(|| {
                anyhow::anyhow!(
                    "set CODE_MODE_SPEC to an OpenAPI 3.x JSON file for adapter `{other}`"
                )
            })?;
            let base_url = base_override
                .ok_or_else(|| anyhow::anyhow!("set CODE_MODE_BASE_URL for adapter `{other}`"))?;
            let token_env = env_or("CODE_MODE_TOKEN_ENV", "API_TOKEN");
            Ok(Box::new(OpenApiAdapter::new(
                other,
                base_url,
                std::fs::read_to_string(path)?,
                Some(required_var(&token_env)?),
                extra,
            )?))
        }
    }
}

fn required_var(name: &str) -> anyhow::Result<String> {
    nonempty_var(name).ok_or_else(|| anyhow::anyhow!("set {name}"))
}

fn nonempty_var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn open_cache() -> anyhow::Result<SqliteStore> {
    match std::env::var("CODE_MODE_CACHE") {
        Ok(value) if value == ":memory:" => Ok(SqliteStore::open_in_memory()?),
        Ok(value) if !value.trim().is_empty() => {
            Ok(SqliteStore::open_path(std::path::Path::new(&value))?)
        }
        _ => Ok(SqliteStore::open_path(&default_cache_path())?),
    }
}

fn default_cache_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("mcp-code-mode").join("cache.db");
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home)
                .join(".cache")
                .join("mcp-code-mode")
                .join("cache.db");
        }
    }
    std::env::temp_dir().join("mcp-code-mode").join("cache.db")
}

fn env_or(name: &str, default: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => default.to_string(),
    }
}

fn env_i64(name: &str, default: i64) -> anyhow::Result<i64> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} must be an integer")),
        _ => Ok(default),
    }
}

fn env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} must be an integer")),
        _ => Ok(default),
    }
}

fn env_u32(name: &str, default: u32) -> anyhow::Result<u32> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .parse()
            .map_err(|_| anyhow::anyhow!("{name} must be an integer")),
        _ => Ok(default),
    }
}

fn parse_extra_headers(raw: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut headers = Vec::new();
    for part in raw.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = part.split_once(':') else {
            anyhow::bail!("CODE_MODE_EXTRA_HEADERS entry `{part}` must be Name:Value");
        };
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("CODE_MODE_EXTRA_HEADERS contains an empty header name");
        }
        headers.push((name.to_string(), value.trim().to_string()));
    }
    Ok(headers)
}
