//! Embedded SQLite persistence for ReverieAgent.
//!
//! The store is a thin, explicit repository layer: every method maps rows by
//! hand so schema drift surfaces as a loud error instead of a silent default.

pub mod attachments;
pub mod blackboard;
pub mod db;
pub mod error;
pub mod evolution;
pub mod models;
pub mod providers;
pub mod agents;
pub mod sessions;
pub mod tools;

use std::path::Path;

use sqlx::SqlitePool;

pub use error::{StoreError, StoreResult};

/// Handle to the embedded database.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    /// Open (creating if needed) the database at `path`, enable WAL and run
    /// every pending migration.
    pub async fn connect(path: impl AsRef<Path>) -> StoreResult<Self> {
        let pool = db::open_pool(path.as_ref()).await?;
        Ok(Store { pool })
    }

    /// Open a private in-memory database. Used by tests.
    pub async fn connect_in_memory() -> StoreResult<Self> {
        let pool = db::open_memory_pool().await?;
        Ok(Store { pool })
    }

    /// Borrow the raw pool for advanced queries.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Flush the WAL back into the main database file.
    pub async fn checkpoint(&self) -> StoreResult<()> {
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&self.pool).await?;
        Ok(())
    }

    /// Delete a session and every row that cascades from it.
    pub async fn delete_session(&self, session_id: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?").bind(session_id).execute(&self.pool).await?;
        Ok(())
    }
}
