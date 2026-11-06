use thiserror::Error;

/// Errors raised by pure-domain logic.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid enum value `{value}` for {kind}")]
    InvalidEnum { kind: &'static str, value: String },

    #[error("serialization failure: {0}")]
    Serialization(String),

    #[error("validation failure: {0}")]
    Validation(String),

    #[error("unsupported protocol: {0}")]
    UnsupportedProtocol(String),

    #[error("not found: {0}")]
    NotFound(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

impl From<serde_json::Error> for CoreError {
    fn from(value: serde_json::Error) -> Self {
        CoreError::Serialization(value.to_string())
    }
}
