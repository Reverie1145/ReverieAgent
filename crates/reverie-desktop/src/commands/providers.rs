//! Provider CRUD commands.

use reverie_core::domain::{Provider, Protocol};
use tauri::State;

use crate::commands::Shared;
use crate::dto::{ProviderInput, ProviderView};

/// List every configured provider with masked credentials.
#[tauri::command]
pub async fn cmd_list_providers(state: State<'_, Shared>) -> Result<Vec<ProviderView>, String> {
    let providers = state.store.list_providers().await.map_err(|err| err.to_string())?;
    let mut out = Vec::with_capacity(providers.len());
    for provider in providers {
        let count = state
            .store
            .list_models_for_provider(&provider.id)
            .await
            .map(|models| models.len() as i64)
            .unwrap_or(0);
        out.push(ProviderView::new(&provider, count));
    }
    Ok(out)
}

/// Create or update a provider.
///
/// When `api_key` is omitted the stored key is preserved, so the UI can edit a
/// provider without ever holding the secret.
#[tauri::command]
pub async fn cmd_upsert_provider(
    state: State<'_, Shared>,
    payload: ProviderInput,
) -> Result<ProviderView, String> {
    let protocol = Protocol::parse(&payload.protocol).map_err(|err| err.to_string())?;

    let existing = match &payload.id {
        Some(id) => state.store.get_provider(id).await.map_err(|err| err.to_string())?,
        None => None,
    };

    let api_key = match payload.api_key.as_deref().map(str::trim) {
        Some(key) if !key.is_empty() => key.to_string(),
        _ => existing.as_ref().map(|provider| provider.api_key.clone()).unwrap_or_default(),
    };

    let mut provider = match existing {
        Some(previous) => Provider { api_key, ..previous },
        None => Provider::create(payload.name.clone(), protocol, payload.base_url.clone(), api_key),
    };

    provider.name = payload.name;
    provider.protocol = protocol;
    provider.base_url = payload.base_url;
    if let Some(enabled) = payload.is_enabled {
        provider.is_enabled = enabled;
    }
    if let Some(config) = payload.search_engine_config {
        provider.search_engine_config_json = config.to_json();
    }

    state.store.upsert_provider(&provider).await.map_err(|err| err.to_string())?;

    let count = state
        .store
        .list_models_for_provider(&provider.id)
        .await
        .map(|models| models.len() as i64)
        .unwrap_or(0);
    Ok(ProviderView::new(&provider, count))
}

/// Delete a provider and cascade its model profiles.
#[tauri::command]
pub async fn cmd_delete_provider(state: State<'_, Shared>, id: String) -> Result<(), String> {
    state.store.delete_provider(&id).await.map_err(|err| err.to_string())
}

/// Update only the search-engine credentials for a provider.
#[tauri::command]
pub async fn cmd_update_provider_search_config(
    state: State<'_, Shared>,
    provider_id: String,
    search_engine_config: reverie_core::domain::SearchEngineConfig,
) -> Result<ProviderView, String> {
    let mut provider = state
        .store
        .get_provider(&provider_id)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("provider {provider_id} not found"))?;

    provider.search_engine_config_json = search_engine_config.to_json();
    state.store.upsert_provider(&provider).await.map_err(|err| err.to_string())?;

    let count = state
        .store
        .list_models_for_provider(&provider.id)
        .await
        .map(|models| models.len() as i64)
        .unwrap_or(0);
    Ok(ProviderView::new(&provider, count))
}
