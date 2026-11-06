//! Session lifecycle commands.

use reverie_core::domain::Session;
use reverie_runtime::snapshot::SessionSnapshot;
use reverie_runtime::CreateSessionRequest;
use tauri::State;

use crate::commands::Shared;

/// Create a session, bind attachments and start the swarm.
#[tauri::command]
pub async fn cmd_create_and_start_session(
    state: State<'_, Shared>,
    title: String,
    workspace_path: String,
    user_objective: String,
    attachment_ids: Vec<String>,
    thinking_mode_override: String,
    web_search_enabled: bool,
    orchestrator_model_id: Option<String>,
) -> Result<Session, String> {
    let request = CreateSessionRequest {
        title,
        workspace_path,
        user_objective,
        attachment_ids,
        thinking_mode_override,
        web_search_enabled,
        orchestrator_model_id,
    };
    state.create_and_start_session(request).await.map_err(|err| err.to_string())
}

/// Recent sessions, newest first.
#[tauri::command]
pub async fn cmd_list_sessions(state: State<'_, Shared>) -> Result<Vec<Session>, String> {
    state.store.list_sessions(200).await.map_err(|err| err.to_string())
}

/// Full snapshot for the task stage.
#[tauri::command]
pub async fn cmd_get_session_snapshot(
    state: State<'_, Shared>,
    session_id: String,
) -> Result<SessionSnapshot, String> {
    state.scheduler.snapshot(&session_id).await.map_err(|err| err.to_string())
}

/// `pause` | `resume` | `cancel`.
#[tauri::command]
pub async fn cmd_control_session(
    state: State<'_, Shared>,
    session_id: String,
    action: String,
) -> Result<String, String> {
    state
        .scheduler
        .control(&session_id, &action)
        .await
        .map_err(|err| err.to_string())
}

/// Delete a session and everything that cascades from it.
#[tauri::command]
pub async fn cmd_delete_session(state: State<'_, Shared>, session_id: String) -> Result<(), String> {
    state.store.delete_session(&session_id).await.map_err(|err| err.to_string())
}

/// Run the evolution engine against a finished session.
#[tauri::command]
pub async fn cmd_reflect_session(
    state: State<'_, Shared>,
    session_id: String,
) -> Result<reverie_runtime::EvolutionReport, String> {
    let sink = state.sink.clone();
    state
        .evolution
        .reflect_session(&session_id, sink.as_ref())
        .await
        .map_err(|err| err.to_string())
}
