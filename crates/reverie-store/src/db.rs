//! Pool construction and defensive row accessors.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};

pub use sqlx::sqlite::SqliteRow;

use crate::error::StoreResult;

/// Open a file-backed pool with WAL enabled and every migration applied.
pub async fn open_pool(db_path: &Path) -> StoreResult<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let options = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(15))
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new().max_connections(16).acquire_timeout(Duration::from_secs(20)).connect_with(options).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Open a single-connection in-memory pool for tests.
pub async fn open_memory_pool() -> StoreResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?
        .journal_mode(SqliteJournalMode::Memory)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// Read a required text column, defaulting to an empty string on type drift.
pub fn s(row: &SqliteRow, column: &str) -> String {
    row.try_get::<String, _>(column).unwrap_or_default()
}

/// Read a nullable text column.
pub fn os(row: &SqliteRow, column: &str) -> Option<String> {
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => value,
        Err(_) => None,
    }
}

/// Read an integer column.
pub fn i(row: &SqliteRow, column: &str) -> i64 {
    row.try_get::<i64, _>(column).unwrap_or(0)
}

/// Read a floating point column.
pub fn f(row: &SqliteRow, column: &str) -> f64 {
    row.try_get::<f64, _>(column).unwrap_or(0.0)
}

/// Read a boolean stored as 0/1.
pub fn b(row: &SqliteRow, column: &str) -> bool {
    i(row, column) != 0
}

/// Parse a stored JSON string array, tolerating NULL and corruption.
pub fn string_array(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Vec::new();
    }
    serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_default()
}

/// Encode a string array for storage.
pub fn encode_string_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

/// Store a boolean as 0/1.
pub fn bool_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_array_handles_garbage() {
        assert_eq!(string_array("[\"a\",\"b\"]"), vec!["a".to_string(), "b".to_string()]);
        assert!(string_array("").is_empty());
        assert!(string_array("null").is_empty());
        assert!(string_array("not json").is_empty());
    }

    #[test]
    fn array_roundtrip() {
        let values = vec!["x".to_string(), "y".to_string()];
        assert_eq!(string_array(&encode_string_array(&values)), values);
    }

    #[test]
    fn bool_mapping() {
        assert_eq!(bool_int(true), 1);
        assert_eq!(bool_int(false), 0);
    }
}
