//! SQLite persistence for the spec index and the TTL response cache.
//!
//! Search uses FTS5 to collect candidates, then [`crate::lexical_score`] to
//! rank them. A query that matches nothing in FTS falls back to a full scan
//! so substring matches still surface.

use crate::score::{fts_document, lexical_score, query_terms};
use crate::IndexError;
use code_mode_core::Operation;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::{Arc, Mutex};

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = r#"
CREATE TABLE schema_version (
    version INTEGER NOT NULL
);
CREATE TABLE spec_meta (
    adapter_id TEXT PRIMARY KEY,
    spec_hash TEXT NOT NULL,
    loaded_at INTEGER NOT NULL
);
CREATE TABLE operations (
    adapter_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    binding_name TEXT NOT NULL,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    summary TEXT NOT NULL,
    description TEXT NOT NULL,
    tags_json TEXT NOT NULL,
    parameters_json TEXT NOT NULL,
    request_schema_json TEXT NOT NULL,
    response_schema_json TEXT NOT NULL,
    PRIMARY KEY (adapter_id, operation_id)
);
CREATE VIRTUAL TABLE operations_fts USING fts5(
    operation_id,
    binding_name,
    path,
    summary,
    description,
    tags,
    adapter_id UNINDEXED,
    tokenize = 'unicode61'
);
CREATE TABLE response_cache (
    cache_key TEXT PRIMARY KEY,
    adapter_id TEXT NOT NULL,
    status INTEGER NOT NULL,
    body TEXT NOT NULL,
    stored_at INTEGER NOT NULL,
    ttl_secs INTEGER NOT NULL
);
"#;

#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedHttp {
    pub adapter_id: String,
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct RankedOperation {
    pub operation: Operation,
    pub score: i32,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

impl SqliteStore {
    pub fn open_in_memory() -> Result<Self, IndexError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    pub fn open_path(path: &Path) -> Result<Self, IndexError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    IndexError::Spec(format!(
                        "failed to create cache directory {}: {error}",
                        parent.display()
                    ))
                })?;
            }
        }
        Self::from_connection(Connection::open(path)?)
    }

    fn from_connection(conn: Connection) -> Result<Self, IndexError> {
        init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, IndexError> {
        self.conn
            .lock()
            .map_err(|poison| IndexError::Spec(format!("sqlite lock poisoned: {poison}")))
    }

    pub fn stored_hash(&self, adapter_id: &str) -> Result<Option<String>, IndexError> {
        let conn = self.lock()?;
        let hash = conn
            .query_row(
                "SELECT spec_hash FROM spec_meta WHERE adapter_id = ?1",
                [adapter_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(hash)
    }

    pub fn replace_operations(
        &self,
        adapter_id: &str,
        spec_hash: &str,
        loaded_at: i64,
        operations: &[Operation],
    ) -> Result<(), IndexError> {
        let mut conn = self.lock()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM operations WHERE adapter_id = ?1", [adapter_id])?;
        tx.execute(
            "DELETE FROM operations_fts WHERE adapter_id = ?1",
            [adapter_id],
        )?;
        for operation in operations {
            let tags_json =
                serde_json::to_string(&operation.tags).map_err(|error| IndexError::Corrupt {
                    operation: operation.operation_id.clone(),
                    message: error.to_string(),
                })?;
            let parameters_json =
                serde_json::to_string(&operation.parameters).map_err(|error| {
                    IndexError::Corrupt {
                        operation: operation.operation_id.clone(),
                        message: error.to_string(),
                    }
                })?;
            let request_json =
                serde_json::to_string(&operation.request_schema).map_err(|error| {
                    IndexError::Corrupt {
                        operation: operation.operation_id.clone(),
                        message: error.to_string(),
                    }
                })?;
            let response_json =
                serde_json::to_string(&operation.response_schema).map_err(|error| {
                    IndexError::Corrupt {
                        operation: operation.operation_id.clone(),
                        message: error.to_string(),
                    }
                })?;
            tx.execute(
                "INSERT INTO operations (
                    adapter_id, operation_id, binding_name, method, path, summary,
                    description, tags_json, parameters_json, request_schema_json,
                    response_schema_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    adapter_id,
                    operation.operation_id,
                    operation.binding_name,
                    operation.method,
                    operation.path,
                    operation.summary,
                    operation.description,
                    tags_json,
                    parameters_json,
                    request_json,
                    response_json,
                ],
            )?;
            let document = fts_document(operation);
            tx.execute(
                "INSERT INTO operations_fts (
                    operation_id, binding_name, path, summary, description, tags, adapter_id
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    operation.operation_id,
                    operation.binding_name,
                    operation.path,
                    operation.summary,
                    operation.description,
                    document,
                    adapter_id,
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO spec_meta (adapter_id, spec_hash, loaded_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(adapter_id) DO UPDATE SET
                spec_hash = excluded.spec_hash,
                loaded_at = excluded.loaded_at",
            params![adapter_id, spec_hash, loaded_at],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn list_operations(&self, adapter_id: &str) -> Result<Vec<Operation>, IndexError> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT operation_id, binding_name, method, path, summary, description,
                    tags_json, parameters_json, request_schema_json, response_schema_json
             FROM operations
             WHERE adapter_id = ?1
             ORDER BY operation_id",
        )?;
        let rows = statement.query_map([adapter_id], row_to_operation)?;
        let mut operations = Vec::new();
        for row in rows {
            operations.push(row?);
        }
        Ok(operations)
    }

    pub fn fts_count(&self, adapter_id: &str) -> Result<i64, IndexError> {
        let conn = self.lock()?;
        let count = conn.query_row(
            "SELECT count(*) FROM operations_fts WHERE adapter_id = ?1",
            [adapter_id],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn search(
        &self,
        adapter_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RankedOperation>, IndexError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let terms = query_terms(query);
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let operations = self.list_operations(adapter_id)?;
        let fts_ids = self.fts_candidates(adapter_id, &terms)?;
        let mut ranked = Vec::new();
        for operation in operations {
            let mut score = lexical_score(&operation, &terms);
            let in_fts = fts_ids.iter().any(|id| id == &operation.operation_id);
            if in_fts {
                score += 1;
            }
            let keep = if fts_ids.is_empty() {
                score > 0
            } else {
                in_fts
            };
            if keep {
                ranked.push(RankedOperation { operation, score });
            }
        }
        ranked.sort_by(|left, right| {
            right.score.cmp(&left.score).then_with(|| {
                left.operation
                    .operation_id
                    .cmp(&right.operation.operation_id)
            })
        });
        ranked.truncate(limit);
        Ok(ranked)
    }

    fn fts_candidates(
        &self,
        adapter_id: &str,
        terms: &[String],
    ) -> Result<Vec<String>, IndexError> {
        let match_query = terms
            .iter()
            .map(|term| format!("{term}*"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT operation_id FROM operations_fts
             WHERE operations_fts MATCH ?1 AND adapter_id = ?2
             ORDER BY bm25(operations_fts)
             LIMIT 50",
        )?;
        let rows = statement.query_map(params![match_query, adapter_id], |row| row.get(0));
        match rows {
            Ok(rows) => {
                let mut ids = Vec::new();
                for row in rows {
                    ids.push(row?);
                }
                Ok(ids)
            }
            Err(_) => Ok(Vec::new()),
        }
    }

    pub fn lookup_cached(&self, key: &str, now: i64) -> Result<Option<CachedHttp>, IndexError> {
        let conn = self.lock()?;
        let row: Option<(String, i64, String, i64, i64)> = conn
            .query_row(
                "SELECT adapter_id, status, body, stored_at, ttl_secs
                 FROM response_cache WHERE cache_key = ?1",
                [key],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((adapter_id, status, body, stored_at, ttl_secs)) = row else {
            return Ok(None);
        };
        if ttl_secs <= 0 || now > stored_at.saturating_add(ttl_secs) {
            drop(conn);
            self.delete_cached(key)?;
            return Ok(None);
        }
        let status = u16::try_from(status).unwrap_or(0);
        Ok(Some(CachedHttp {
            adapter_id,
            status,
            body,
        }))
    }

    pub fn store_cached(
        &self,
        key: &str,
        response: &CachedHttp,
        now: i64,
        ttl_secs: i64,
    ) -> Result<(), IndexError> {
        if ttl_secs <= 0 {
            return Ok(());
        }
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO response_cache (cache_key, adapter_id, status, body, stored_at, ttl_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(cache_key) DO UPDATE SET
                adapter_id = excluded.adapter_id,
                status = excluded.status,
                body = excluded.body,
                stored_at = excluded.stored_at,
                ttl_secs = excluded.ttl_secs",
            params![
                key,
                response.adapter_id,
                i64::from(response.status),
                response.body,
                now,
                ttl_secs,
            ],
        )?;
        Ok(())
    }

    fn delete_cached(&self, key: &str) -> Result<(), IndexError> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM response_cache WHERE cache_key = ?1", [key])?;
        Ok(())
    }
}

fn init_schema(conn: &Connection) -> Result<(), IndexError> {
    let ready = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .optional()
        .unwrap_or(None);
    if ready == Some(SCHEMA_VERSION) {
        return Ok(());
    }
    conn.execute_batch(
        "DROP TABLE IF EXISTS response_cache;
         DROP TABLE IF EXISTS operations;
         DROP TABLE IF EXISTS spec_meta;
         DROP TABLE IF EXISTS operations_fts;
         DROP TABLE IF EXISTS schema_version;",
    )?;
    conn.execute_batch(SCHEMA)?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        [SCHEMA_VERSION],
    )?;
    Ok(())
}

fn row_to_operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<Operation> {
    let operation_id: String = row.get(0)?;
    let tags_json: String = row.get(6)?;
    let parameters_json: String = row.get(7)?;
    let request_json: String = row.get(8)?;
    let response_json: String = row.get(9)?;
    let tags = serde_json::from_str(&tags_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let parameters = serde_json::from_str(&parameters_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let request_schema: Value = serde_json::from_str(&request_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let response_schema: Value = serde_json::from_str(&response_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Operation {
        operation_id,
        binding_name: row.get(1)?,
        method: row.get(2)?,
        path: row.get(3)?,
        summary: row.get(4)?,
        description: row.get(5)?,
        tags,
        parameters,
        request_schema,
        response_schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ensure_indexed;
    use std::time::{SystemTime, UNIX_EPOCH};

    const SPEC: &str = r#"{
        "openapi": "3.0.3",
        "info": {"title": "Demo", "version": "1"},
        "paths": {
            "/v1/pages": {
                "post": {
                    "operationId": "create_page",
                    "summary": "Create a page",
                    "description": "Create a child page",
                    "tags": ["Pages"],
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/v1/pages/{page_id}": {
                "patch": {
                    "operationId": "update_page",
                    "summary": "Update a page",
                    "description": "Edit page properties",
                    "tags": ["Pages"],
                    "parameters": [
                        {"name": "page_id", "in": "path", "required": true, "schema": {"type": "string"}}
                    ],
                    "responses": {"200": {"description": "ok"}}
                }
            },
            "/v1/databases": {
                "post": {
                    "operationId": "create_database",
                    "summary": "Create a database",
                    "description": "Create a database under a page",
                    "tags": ["Databases"],
                    "responses": {"200": {"description": "ok"}}
                }
            }
        }
    }"#;

    #[test]
    fn index_roundtrip_search_and_hash_reuse() {
        let store = SqliteStore::open_in_memory().unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let first = ensure_indexed(&store, "notion", SPEC, now).unwrap();
        assert_eq!(first.len(), 3);
        assert!(store.fts_count("notion").unwrap() >= 3);
        let second = ensure_indexed(&store, "notion", SPEC, now + 5).unwrap();
        assert_eq!(first, second);
        let hits = store.search("notion", "create page", 2).unwrap();
        assert_eq!(hits[0].operation.operation_id, "create_page");
        assert!(hits
            .iter()
            .any(|hit| hit.operation.operation_id == "create_page"));
        let db = store.search("notion", "create database", 1).unwrap();
        assert_eq!(db[0].operation.operation_id, "create_database");
    }

    #[test]
    fn response_cache_respects_ttl() {
        let store = SqliteStore::open_in_memory().unwrap();
        let cached = CachedHttp {
            adapter_id: "notion".into(),
            status: 200,
            body: "{\"ok\":true}".into(),
        };
        store.store_cached("key", &cached, 1_000, 10).unwrap();
        assert_eq!(
            store.lookup_cached("key", 1_005).unwrap().unwrap().body,
            cached.body
        );
        assert!(store.lookup_cached("key", 1_011).unwrap().is_none());
        assert!(store.lookup_cached("key", 1_012).unwrap().is_none());
    }
}
