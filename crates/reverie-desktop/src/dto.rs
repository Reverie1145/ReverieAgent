//! Data-transfer objects shared with the React frontend.

use chrono::{DateTime, Utc};
use reverie_core::domain::{Provider, SearchEngineConfig, SessionAttachment};
use serde::{Deserialize, Serialize};

/// A provider as the UI sees it: the API key is masked.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderView {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key_masked: String,
    pub has_api_key: bool,
    pub is_enabled: bool,
    pub search_engine_config: SearchEngineConfig,
    pub model_count: i64,
    pub created_at: DateTime<Utc>,
}

impl ProviderView {
    pub fn new(provider: &Provider, model_count: i64) -> Self {
        ProviderView {
            id: provider.id.clone(),
            name: provider.name.clone(),
            protocol: provider.protocol.as_str().to_string(),
            base_url: provider.base_url.clone(),
            api_key_masked: provider.redacted_api_key(),
            has_api_key: !provider.api_key.trim().is_empty(),
            is_enabled: provider.is_enabled,
            search_engine_config: provider.search_config(),
            model_count,
            created_at: provider.created_at,
        }
    }
}

/// Create/update payload for a provider.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub is_enabled: Option<bool>,
    #[serde(default)]
    pub search_engine_config: Option<SearchEngineConfig>,
}

/// Result of staging attachments.
#[derive(Debug, Clone, Serialize)]
pub struct StageResult {
    pub attachments: Vec<SessionAttachment>,
    pub errors: Vec<String>,
}

/// Static application information for the settings footer.
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub db_path: String,
    pub default_workspace: String,
    pub tool_count: usize,
    pub model_count: i64,
    pub session_count: i64,
    pub provider_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::Protocol;

    #[test]
    fn provider_view_masks_the_key() {
        let provider = Provider::create("OpenAI", Protocol::OpenAiCompat, "https://api.openai.com/v1", "sk-abcdefghijkl");
        let view = ProviderView::new(&provider, 12);
        assert_eq!(view.api_key_masked, "sk-a…ijkl");
        assert!(view.has_api_key);
        assert_eq!(view.model_count, 12);
        assert_eq!(view.protocol, "openai_compat");
    }

    #[test]
    fn provider_input_tolerates_missing_optional_fields() {
        let input: ProviderInput = serde_json::from_str(
            r#"{"name":"X","protocol":"ollama","base_url":"http://127.0.0.1:11434"}"#,
        )
        .unwrap();
        assert!(input.id.is_none());
        assert!(input.api_key.is_none());
        assert!(input.search_engine_config.is_none());
    }
}
