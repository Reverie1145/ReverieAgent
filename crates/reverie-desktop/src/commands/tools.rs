//! Toolbox inspection commands.

use reverie_core::domain::ToolDescriptor;
use tauri::State;

use crate::commands::Shared;

/// Every callable tool: the twelve primitives, the meta tools, and anything the
/// swarm has forged so far.
#[tauri::command]
pub async fn cmd_list_all_tools(state: State<'_, Shared>) -> Result<Vec<ToolDescriptor>, String> {
    Ok(state.registry.descriptors())
}
