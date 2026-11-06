//! Error taxonomy for the LLM gateway.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("provider returned {status}: {body}")]
    Provider { status: u16, body: String },

    #[error("protocol violation: {0}")]
    Protocol(String),

    #[error("no model satisfies the request ({0})")]
    NoModel(String),

    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("operation cancelled")]
    Cancelled,

    #[error("{0}")]
    Other(String),
}

pub type LlmResult<T> = Result<T, LlmError>;

impl LlmError {
    /// Whether retrying the same request could plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            LlmError::Http(err) => err.is_timeout() || err.is_connect(),
            LlmError::Provider { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }

    /// HTTP status when the failure came from the provider.
    pub fn status(&self) -> Option<u16> {
        match self {
            LlmError::Provider { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// True when the provider rejected the payload shape (as opposed to auth or
    /// rate limiting). The capability prober relies on this distinction.
    pub fn is_payload_rejection(&self) -> bool {
        matches!(self.status(), Some(400) | Some(422))
    }

    pub fn provider(status: u16, body: impl Into<String>) -> Self {
        LlmError::Provider { status, body: truncate_body(body.into()) }
    }
}

fn truncate_body(body: String) -> String {
    const LIMIT: usize = 800;
    if body.chars().count() <= LIMIT {
        body
    } else {
        let head: String = body.chars().take(LIMIT).collect();
        format!("{head}…")
    }
}

/// Turn a non-success response into a typed error.
pub async fn ensure_success(response: reqwest::Response) -> LlmResult<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    Err(LlmError::provider(status, body))
}
