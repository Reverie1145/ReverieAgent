//! Evolution store commands.

use reverie_core::domain::EvolutionRecord;
use tauri::State;

use crate::commands::Shared;

/// Every versioned role prompt and playbook.
#[tauri::command]
pub async fn cmd_list_evolution_records(
    state: State<'_, Shared>,
) -> Result<Vec<EvolutionRecord>, String> {
    state.store.list_evolution_records().await.map_err(|err| err.to_string())
}

/// Save an edited record, bumping its version.
#[tauri::command]
pub async fn cmd_save_evolution_record(
    state: State<'_, Shared>,
    id: String,
    content: String,
) -> Result<EvolutionRecord, String> {
    if content.trim().is_empty() {
        return Err("content must not be empty".to_string());
    }
    state
        .store
        .evolve_record_content(&id, &content, "manual edit from the Skills & Evolution view")
        .await
        .map_err(|err| err.to_string())
}
