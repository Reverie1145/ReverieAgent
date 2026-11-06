//! Sub-agent inspection and live re-routing commands.

use reverie_core::llm::ThinkingLevel;
use reverie_runtime::snapshot::AgentDetail;
use tauri::State;

use crate::commands::Shared;

/// Switch a running agent's model and/or thinking depth.
#[tauri::command]
pub async fn cmd_update_agent_thinking_or_model(
    state: State<'_, Shared>,
    agent_id: String,
    new_model_id: Option<String>,
    new_thinking_level: Option<String>,
) -> Result<(), String> {
    let model = match new_model_id {
        Some(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    };
    let thinking = match new_thinking_level {
        Some(value) if !value.trim().is_empty() && value != "auto" => {
            Some(ThinkingLevel::parse(&value).map_err(|err| err.to_string())?)
        }
        _ => None,
    };
    if model.is_none() && thinking.is_none() {
        return Err("nothing to change: provide a model id or a thinking level".to_string());
    }
    state
        .scheduler
        .reroute(&agent_id, model, thinking)
        .await
        .map_err(|err| err.to_string())
}

/// Everything the inspector drawer shows.
#[tauri::command]
pub async fn cmd_get_agent_detail(
    state: State<'_, Shared>,
    agent_id: String,
) -> Result<AgentDetail, String> {
    state.scheduler.agent_detail(&agent_id).await.map_err(|err| err.to_string())
}
