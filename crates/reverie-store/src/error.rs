//! Store error taxonomy.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sql(#[from] sqlx::Error),

    #[error("migration error: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("row not found: {0}")]
    NotFound(String),

    #[error("invalid stored data: {0}")]
    Invalid(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

impl StoreError {
    pub fn not_found(what: impl std::fmt::Display) -> Self {
        StoreError::NotFound(what.to_string())
    }

    pub fn invalid(what: impl std::fmt::Display) -> Self {
        StoreError::Invalid(what.to_string())
    }
}
