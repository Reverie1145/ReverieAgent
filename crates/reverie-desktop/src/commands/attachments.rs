//! Attachment staging command.

use std::path::PathBuf;

use reverie_runtime::AttachmentStager;
use tauri::State;

use crate::commands::Shared;
use crate::dto::StageResult;

/// Copy user files into the workspace, generating previews and text extracts.
///
/// Rows are persisted with an empty `session_id`; they are bound to a session
/// when the task is started.
#[tauri::command]
pub async fn cmd_stage_attachments(
    state: State<'_, Shared>,
    file_paths: Vec<String>,
    workspace_path: String,
) -> Result<StageResult, String> {
    if file_paths.is_empty() {
        return Ok(StageResult { attachments: Vec::new(), errors: Vec::new() });
    }
    if workspace_path.trim().is_empty() {
        return Err("select a workspace folder before attaching files".to_string());
    }

    let workspace = PathBuf::from(&workspace_path);
    std::fs::create_dir_all(&workspace).map_err(|err| format!("cannot create workspace: {err}"))?;

    let (attachments, errors) = AttachmentStager::stage_batch(&file_paths, &workspace).await;
    if !attachments.is_empty() {
        // Staged rows live in memory until a session claims them, so the
        // session_attachments foreign key is never violated.
        state.stage_attachments(attachments.clone());
    }

    Ok(StageResult { attachments, errors })
}

/// Stage an in-memory payload (clipboard paste, browser drag payload).
///
/// The bytes are written into `<workspace>/.reverie/attachments` so the agent
/// can inspect them later, exactly like a file picked from disk.
#[tauri::command]
pub async fn cmd_stage_attachment_bytes(
    state: State<'_, Shared>,
    file_name: String,
    mime_type: String,
    base64_data: String,
    workspace_path: String,
) -> Result<reverie_core::domain::SessionAttachment, String> {
    if workspace_path.trim().is_empty() {
        return Err("select a workspace folder before attaching files".to_string());
    }
    let bytes = reverie_core::util::decode_base64(&base64_data)
        .map_err(|err| format!("invalid base64 payload: {err}"))?;

    let workspace = PathBuf::from(&workspace_path);
    std::fs::create_dir_all(&workspace).map_err(|err| format!("cannot create workspace: {err}"))?;

    let safe_name = if file_name.trim().is_empty() { "pasted-file".to_string() } else { file_name };
    let attachment = AttachmentStager::stage_bytes(&safe_name, &mime_type, &bytes, &workspace)?;
    state.stage_attachments(vec![attachment.clone()]);
    Ok(attachment)
}
