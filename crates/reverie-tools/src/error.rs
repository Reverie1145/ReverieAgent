//! Tool error taxonomy.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("denied: {0}")]
    Denied(String),

    #[error("timed out after {0} ms")]
    Timeout(u64),

    #[error("command exited with {code}: {stderr}")]
    Command { code: i32, stderr: String },

    #[error("store error: {0}")]
    Store(String),

    #[error("capability unavailable: {0}")]
    Unavailable(String),

    #[error("{0}")]
    Other(String),
}

pub type ToolResult<T> = Result<T, ToolError>;

impl ToolError {
    pub fn invalid(message: impl std::fmt::Display) -> Self {
        ToolError::InvalidArgs(message.to_string())
    }

    pub fn other(message: impl std::fmt::Display) -> Self {
        ToolError::Other(message.to_string())
    }

    pub fn unavailable(message: impl std::fmt::Display) -> Self {
        ToolError::Unavailable(message.to_string())
    }
}

impl From<reverie_store::StoreError> for ToolError {
    fn from(value: reverie_store::StoreError) -> Self {
        ToolError::Store(value.to_string())
    }
}

impl From<reverie_core::CoreError> for ToolError {
    fn from(value: reverie_core::CoreError) -> Self {
        ToolError::Other(value.to_string())
    }
}
