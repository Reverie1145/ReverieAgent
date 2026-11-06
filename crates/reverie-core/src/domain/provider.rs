//! LLM provider configuration and search-engine credentials.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};
use crate::time;

/// Wire protocol spoken by a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// OpenAI `/v1/chat/completions` shape, used by OpenAI, DeepSeek, Groq,
    /// Together, vLLM, LM Studio, OpenRouter, Moonshot, Qwen, …
    OpenAiCompat,
    /// Anthropic Messages API.
    Anthropic,
    /// Google Generative Language API.
    Gemini,
    /// Local Ollama daemon.
    Ollama,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::OpenAiCompat => "openai_compat",
            Protocol::Anthropic => "anthropic",
            Protocol::Gemini => "gemini",
            Protocol::Ollama => "ollama",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai_compat" | "openai" | "openai-compatible" => Ok(Protocol::OpenAiCompat),
            "anthropic" | "claude" => Ok(Protocol::Anthropic),
            "gemini" | "google" => Ok(Protocol::Gemini),
            "ollama" | "local" => Ok(Protocol::Ollama),
            other => Err(CoreError::UnsupportedProtocol(other.to_string())),
        }
    }

    /// Default base URL used when the user leaves the field blank.
    pub fn default_base_url(self) -> &'static str {
        match self {
            Protocol::OpenAiCompat => "https://api.openai.com/v1",
            Protocol::Anthropic => "https://api.anthropic.com/v1",
            Protocol::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Protocol::Ollama => "http://127.0.0.1:11434",
        }
    }

    /// Whether an API key is mandatory for this protocol.
    pub fn requires_api_key(self) -> bool {
        !matches!(self, Protocol::Ollama)
    }
}

/// Optional third-party search credentials. When every field is empty the
/// `web_search` tool falls back to keyless DuckDuckGo/Bing HTML scraping.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchEngineConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tavily_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brave_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serper_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub searxng_endpoint: Option<String>,
    /// `auto` (default), or a forced engine name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_engine: Option<String>,
}

impl SearchEngineConfig {
    pub fn from_json(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "null" {
            return SearchEngineConfig::default();
        }
        serde_json::from_str(trimmed).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn has_api_backend(&self) -> bool {
        self.tavily_api_key.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false)
            || self.brave_api_key.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false)
            || self.serper_api_key.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false)
            || self.searxng_endpoint.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false)
    }

    /// Names of the configured backends, in priority order.
    pub fn available_backends(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.tavily_api_key.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false) {
            out.push("tavily");
        }
        if self.brave_api_key.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false) {
            out.push("brave");
        }
        if self.serper_api_key.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false) {
            out.push("serper");
        }
        if self.searxng_endpoint.as_deref().map(|v| !v.trim().is_empty()).unwrap_or(false) {
            out.push("searxng");
        }
        out.push("duckduckgo");
        out
    }
}

/// A configured LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub search_engine_config_json: String,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
}

impl Provider {
    /// Build a new provider with a generated id.
    pub fn create(
        name: impl Into<String>,
        protocol: Protocol,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let name = name.into();
        let base_url = base_url.into();
        Provider {
            id: Uuid::new_v4().to_string(),
            name,
            protocol,
            base_url: if base_url.trim().is_empty() {
                protocol.default_base_url().to_string()
            } else {
                base_url
            },
            api_key: api_key.into(),
            search_engine_config_json: "{}".to_string(),
            is_enabled: true,
            created_at: time::now(),
        }
    }

    pub fn search_config(&self) -> SearchEngineConfig {
        SearchEngineConfig::from_json(&self.search_engine_config_json)
    }

    /// Normalized base URL without a trailing slash.
    pub fn normalized_base_url(&self) -> String {
        self.base_url.trim().trim_end_matches('/').to_string()
    }

    /// Validate before persisting.
    pub fn validate(&self) -> CoreResult<()> {
        if self.name.trim().is_empty() {
            return Err(CoreError::Validation("provider name must not be empty".into()));
        }
        if self.base_url.trim().is_empty() {
            return Err(CoreError::Validation("base_url must not be empty".into()));
        }
        if !self.base_url.starts_with("http://") && !self.base_url.starts_with("https://") {
            return Err(CoreError::Validation("base_url must start with http:// or https://".into()));
        }
        if self.protocol.requires_api_key() && self.api_key.trim().is_empty() {
            return Err(CoreError::Validation(format!(
                "protocol `{}` requires an API key",
                self.protocol.as_str()
            )));
        }
        Ok(())
    }

    /// API key with the middle masked, safe to send to the UI.
    ///
    /// Works on characters, not bytes: an API key field can legitimately hold
    /// any Unicode the user pasted, and byte slicing would panic mid-character.
    pub fn redacted_api_key(&self) -> String {
        let key = self.api_key.trim();
        let total = key.chars().count();
        if total <= 8 {
            return "*".repeat(total);
        }
        let head = crate::util::char_prefix(key, 4);
        let tail: String = key.chars().skip(total - 4).collect();
        format!("{head}…{tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_roundtrip_and_aliases() {
        for proto in [Protocol::OpenAiCompat, Protocol::Anthropic, Protocol::Gemini, Protocol::Ollama] {
            assert_eq!(Protocol::parse(proto.as_str()).unwrap(), proto);
        }
        assert_eq!(Protocol::parse("Claude").unwrap(), Protocol::Anthropic);
        assert!(Protocol::parse("grpc").is_err());
    }

    #[test]
    fn blank_base_url_falls_back_to_default() {
        let p = Provider::create("OpenAI", Protocol::OpenAiCompat, "  ", "sk-1234567890");
        assert_eq!(p.normalized_base_url(), "https://api.openai.com/v1");
    }

    #[test]
    fn validation_rejects_missing_key_for_remote_protocols() {
        let mut p = Provider::create("X", Protocol::Anthropic, "https://api.anthropic.com/v1", "");
        assert!(p.validate().is_err());
        p.api_key = "sk-ant-1".into();
        assert!(p.validate().is_ok());
        let local = Provider::create("Local", Protocol::Ollama, "http://127.0.0.1:11434", "");
        assert!(local.validate().is_ok());
    }

    #[test]
    fn api_key_is_masked() {
        let p = Provider::create("X", Protocol::OpenAiCompat, "https://x", "sk-abcdefghijkl");
        assert_eq!(p.redacted_api_key(), "sk-a…ijkl");
    }

    #[test]
    fn masking_never_panics_on_non_ascii_keys() {
        // Regression: a pasted Chinese string used to panic on a byte slice.
        let p = Provider::create(
            "X",
            Protocol::OpenAiCompat,
            "https://x",
            "使用 网易UU远程 即可对我发起远程协助",
        );
        let masked = p.redacted_api_key();
        assert!(masked.starts_with("使用 网"));
        assert!(masked.contains('…'));
        assert_eq!(masked.chars().count(), 9);
    }

    #[test]
    fn short_keys_are_fully_masked() {
        let p = Provider::create("X", Protocol::OpenAiCompat, "https://x", "abc");
        assert_eq!(p.redacted_api_key(), "***");
        let empty = Provider::create("X", Protocol::Ollama, "http://127.0.0.1:11434", "");
        assert_eq!(empty.redacted_api_key(), "");
    }

    #[test]
    fn search_config_detects_backends() {
        let cfg = SearchEngineConfig::from_json("{\"tavily_api_key\":\"tvly-1\"}");
        assert!(cfg.has_api_backend());
        assert_eq!(cfg.available_backends(), vec!["tavily", "duckduckgo"]);
        let empty = SearchEngineConfig::from_json("garbage");
        assert!(!empty.has_api_backend());
        assert_eq!(empty.available_backends(), vec!["duckduckgo"]);
    }
}
