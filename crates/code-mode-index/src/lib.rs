//! OpenAPI loading and the SQLite spec / response cache.

mod openapi;
mod score;
mod store;

pub use openapi::parse_openapi;
pub use score::{lexical_score, query_terms};
pub use store::{sha256_hex, CachedHttp, RankedOperation, SqliteStore};

use code_mode_core::Operation;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("OpenAPI parse error: {0}")]
    Spec(String),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stored operation `{operation}` is corrupt: {message}")]
    Corrupt { operation: String, message: String },
}

/// Parse `spec_json` and replace this adapter's rows when the hash changed.
pub fn ensure_indexed(
    store: &SqliteStore,
    adapter_id: &str,
    spec_json: &str,
    loaded_at: i64,
) -> Result<Vec<Operation>, IndexError> {
    let hash = sha256_hex(spec_json.as_bytes());
    if store.stored_hash(adapter_id)? == Some(hash.clone()) {
        return store.list_operations(adapter_id);
    }
    let operations = parse_openapi(spec_json)?;
    store.replace_operations(adapter_id, &hash, loaded_at, &operations)?;
    Ok(operations)
}
