//! Application metadata command.

use tauri::State;

use crate::commands::Shared;
use crate::dto::AppInfo;

/// Version, storage location and counters, for the sidebar footer.
#[tauri::command]
pub async fn cmd_get_app_info(state: State<'_, Shared>) -> Result<AppInfo, String> {
    let default_workspace = dirs_default_workspace();
    Ok(AppInfo {
        name: "ReverieAgent".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        db_path: state.db_path.to_string_lossy().to_string(),
        default_workspace,
        tool_count: state.registry.len(),
        model_count: state.store.count_models().await.map_err(|err| err.to_string())?,
        session_count: state.store.count_sessions().await.map_err(|err| err.to_string())?,
        provider_count: state.store.count_providers().await.map_err(|err| err.to_string())?,
    })
}

/// A sensible starting folder for the workspace picker.
fn dirs_default_workspace() -> String {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(|home| format!("{home}/ReverieWorkspace"))
        .unwrap_or_else(|_| ".".to_string())
}
