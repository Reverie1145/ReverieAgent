//! Multimodal attachment inspection.

use std::path::Path;

use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reverie_core::domain::{SessionAttachment, ToolOutcome};
use reverie_core::util::AttachmentKind;
use serde_json::{json, Value};

use crate::args::{opt_i64_clamped, opt_str};
use crate::context::ToolExecutionContext;
use crate::error::{ToolError, ToolResult};
use crate::registry::{AgentTool, ToolCategory};

/// Largest file that will be base64-encoded for a vision request.
const MAX_VISION_BYTES: u64 = 12 * 1024 * 1024;
/// Default text preview size.
const DEFAULT_TEXT_CHARS: i64 = 20_000;

/// Inspect a file attached to the session or produced in the workspace.
pub struct InspectAttachmentTool;

struct Resolved {
    file_name: String,
    file_path: String,
    mime_type: String,
    size: u64,
    preview: Option<String>,
}

async fn resolve(ctx: &ToolExecutionContext, args: &Value) -> ToolResult<Resolved> {
    if let Some(id) = opt_str(args, "attachment_id") {
        let attachment: SessionAttachment = ctx
            .store
            .get_attachment(&id)
            .await?
            .ok_or_else(|| ToolError::NotFound(format!("attachment {id}")))?;
        let size = std::fs::metadata(&attachment.file_path).map(|m| m.len()).unwrap_or(0);
        return Ok(Resolved {
            file_name: attachment.file_name,
            file_path: attachment.file_path,
            mime_type: attachment.mime_type,
            size,
            preview: attachment.extracted_text_preview,
        });
    }

    let raw = opt_str(args, "file_path")
        .ok_or_else(|| ToolError::invalid("provide either `attachment_id` or `file_path`"))?;
    let path = ctx.resolve(&raw);
    if !path.exists() {
        return Err(ToolError::NotFound(format!("{}", path.display())));
    }
    let metadata = std::fs::metadata(&path)?;
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| raw.clone());
    let mime_type = reverie_core::util::guess_mime(&file_name);
    Ok(Resolved { file_name, file_path: path.to_string_lossy().to_string(), mime_type, size: metadata.len(), preview: None })
}

fn read_text_body(path: &Path, max_chars: usize) -> ToolResult<String> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    Ok(reverie_core::util::truncate_middle(&text, max_chars))
}

#[async_trait]
impl AgentTool for InspectAttachmentTool {
    fn name(&self) -> &str {
        "inspect_attachment"
    }

    fn description(&self) -> &str {
        "Inspect a session attachment or workspace file. mode=metadata returns size and type, \
         mode=text returns extracted text (works for code, CSV, Markdown, JSON), and mode=vision \
         returns the image inline so a vision-capable model can actually look at it."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "attachment_id": {"type": "string", "description": "Id from session_attachments"},
                "file_path": {"type": "string", "description": "Workspace path, used when no id is given"},
                "mode": {
                    "type": "string",
                    "enum": ["metadata", "text", "vision"],
                    "default": "metadata"
                },
                "max_chars": {"type": "integer", "description": "Text cap", "default": DEFAULT_TEXT_CHARS}
            },
            "required": []
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Context
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let resolved = resolve(ctx, &args).await?;
        let mode = opt_str(&args, "mode").unwrap_or_else(|| "metadata".to_string());
        let max_chars = opt_i64_clamped(&args, "max_chars", DEFAULT_TEXT_CHARS, 500, 200_000) as usize;
        let kind = AttachmentKind::classify(&resolved.mime_type, &resolved.file_name);

        match mode.as_str() {
            "vision" => {
                if !kind.is_vision_capable() {
                    return Ok(ToolOutcome::error(format!(
                        "{} is {} and cannot be rendered as an image; use mode=text instead",
                        resolved.file_name, resolved.mime_type
                    )));
                }
                if resolved.size > MAX_VISION_BYTES {
                    return Ok(ToolOutcome::error(format!(
                        "{} is {} bytes, above the {} byte vision limit",
                        resolved.file_name, resolved.size, MAX_VISION_BYTES
                    )));
                }
                let bytes = std::fs::read(&resolved.file_path)?;
                let encoded = STANDARD.encode(&bytes);
                ctx.emit_activity(format!("Inspecting image {}", resolved.file_name));
                Ok(ToolOutcome::ok_with(
                    format!(
                        "Attached image `{}` ({}, {} bytes) for visual inspection.",
                        resolved.file_name, resolved.mime_type, resolved.size
                    ),
                    json!({
                        "image_base64": encoded,
                        "mime_type": resolved.mime_type,
                        "file_name": resolved.file_name,
                    }),
                ))
            }
            "text" => {
                if kind == AttachmentKind::Image {
                    return Ok(ToolOutcome::error(format!(
                        "{} is an image; call this tool again with mode=vision",
                        resolved.file_name
                    )));
                }
                if let Some(preview) = &resolved.preview {
                    return Ok(ToolOutcome::ok(ctx.cap_output(format!(
                        "{} ({}):\n\n{}",
                        resolved.file_name, resolved.mime_type, preview
                    ))));
                }
                let body = read_text_body(Path::new(&resolved.file_path), max_chars)?;
                if body.trim().is_empty() {
                    return Ok(ToolOutcome::error(format!(
                        "{} contains no readable text ({} bytes of binary data)",
                        resolved.file_name, resolved.size
                    )));
                }
                Ok(ToolOutcome::ok_with(
                    ctx.cap_output(format!("{} ({}):\n\n{}", resolved.file_name, resolved.mime_type, body)),
                    json!({"chars": body.chars().count()}),
                ))
            }
            _ => Ok(ToolOutcome::ok_with(
                format!(
                    "{} · {} · {} bytes · kind={:?} · has_text_preview={}",
                    resolved.file_name,
                    resolved.mime_type,
                    resolved.size,
                    kind,
                    resolved.preview.is_some()
                ),
                json!({
                    "file_name": resolved.file_name,
                    "mime_type": resolved.mime_type,
                    "size_bytes": resolved.size,
                    "file_path": resolved.file_path,
                }),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::events::NoopSink;
    use std::sync::Arc;

    async fn context(dir: &Path) -> ToolExecutionContext {
        let store = Arc::new(reverie_store::Store::connect_in_memory().await.unwrap());
        let session = reverie_core::domain::Session::create("t", dir.to_string_lossy(), "o", "auto", true);
        store.insert_session(&session).await.unwrap();
        ToolExecutionContext {
            session_id: session.id,
            agent_id: "a".into(),
            role_name: "tester".into(),
            workspace_dir: dir.to_path_buf(),
            store,
            vfs: Arc::new(crate::vfs::VfsLockManager::new()),
            sink: Arc::new(NoopSink),
            http: reqwest::Client::new(),
            search_config: reverie_core::domain::SearchEngineConfig::default(),
            web_search_enabled: true,
            registry: Arc::new(crate::registry::GlobalToolRegistry::new()),
            orchestration: None,
            max_output_chars: 20_000,
        }
    }

    fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("reverie-attach-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn text_mode_reads_code_files() {
        let dir = temp_dir();
        std::fs::write(dir.join("lib.rs"), "pub fn x() {}\n").unwrap();
        let ctx = context(&dir).await;
        let outcome = InspectAttachmentTool
            .execute(&ctx, json!({"file_path": "lib.rs", "mode": "text"}))
            .await
            .unwrap();
        assert!(outcome.content.contains("pub fn x()"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn vision_mode_returns_base64() {
        let dir = temp_dir();
        let png = reverie_core::util::decode_base64(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
        )
        .unwrap();
        std::fs::write(dir.join("shot.png"), png).unwrap();
        let ctx = context(&dir).await;
        let outcome = InspectAttachmentTool
            .execute(&ctx, json!({"file_path": "shot.png", "mode": "vision"}))
            .await
            .unwrap();
        assert!(outcome.metadata["image_base64"].as_str().unwrap().len() > 10);
        assert_eq!(outcome.metadata["mime_type"], "image/png");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn text_mode_rejects_images() {
        let dir = temp_dir();
        std::fs::write(dir.join("a.png"), [0x89u8, 0x50, 0x4E, 0x47]).unwrap();
        let ctx = context(&dir).await;
        let outcome = InspectAttachmentTool
            .execute(&ctx, json!({"file_path": "a.png", "mode": "text"}))
            .await
            .unwrap();
        assert!(outcome.is_error);
        assert!(outcome.content.contains("mode=vision"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn metadata_mode_describes_file() {
        let dir = temp_dir();
        std::fs::write(dir.join("data.csv"), "a,b\n1,2\n").unwrap();
        let ctx = context(&dir).await;
        let outcome = InspectAttachmentTool
            .execute(&ctx, json!({"file_path": "data.csv"}))
            .await
            .unwrap();
        assert!(outcome.content.contains("data.csv"));
        assert!(outcome.content.contains("text/csv"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn missing_arguments_error() {
        let dir = temp_dir();
        let ctx = context(&dir).await;
        assert!(InspectAttachmentTool.execute(&ctx, json!({})).await.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
