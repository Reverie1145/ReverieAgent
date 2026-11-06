//! Model discovery and profiling commands.

use reverie_core::domain::ModelProfile;
use tauri::State;

use crate::commands::Shared;

/// Pull the provider's live model list and sync `model_profiles`.
#[tauri::command]
pub async fn cmd_pull_provider_models(
    state: State<'_, Shared>,
    provider_id: String,
) -> Result<Vec<ModelProfile>, String> {
    let sink = state.sink.clone();
    state
        .profiler
        .pull_models(&provider_id, sink.as_ref())
        .await
        .map_err(|err| err.to_string())
}

/// Run the concurrent capability probe plus the evaluation agent.
#[tauri::command]
pub async fn cmd_profile_provider_models(
    state: State<'_, Shared>,
    provider_id: Option<String>,
) -> Result<Vec<ModelProfile>, String> {
    let sink = state.sink.clone();
    match provider_id {
        Some(id) => state
            .profiler
            .profile_provider(&id, sink.as_ref())
            .await
            .map_err(|err| err.to_string()),
        None => state.profiler.profile_all(sink.as_ref()).await.map_err(|err| err.to_string()),
    }
}

/// Update a model's default thinking depth and cost tier.
#[tauri::command]
pub async fn cmd_update_model_profile_settings(
    state: State<'_, Shared>,
    model_id: String,
    default_thinking_level: String,
    cost_tier: i64,
) -> Result<ModelProfile, String> {
    state
        .store
        .update_model_settings(&model_id, &default_thinking_level, cost_tier)
        .await
        .map_err(|err| err.to_string())
}

/// Every profiled model across all providers.
#[tauri::command]
pub async fn cmd_list_model_profiles(state: State<'_, Shared>) -> Result<Vec<ModelProfile>, String> {
    state.store.list_model_profiles().await.map_err(|err| err.to_string())
}
