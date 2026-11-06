//! Attachment staging: copy user files into the session workspace, generate
//! previews, and extract text so non-vision models can still read them.

use std::path::{Path, PathBuf};

use reverie_core::domain::SessionAttachment;
use reverie_core::util::{self, AttachmentKind};
use reverie_core::RUNTIME_DIR;

/// Images larger than this are not inlined as previews.
const MAX_THUMBNAIL_BYTES: u64 = 2 * 1024 * 1024;
/// Characters captured for the text preview.
const PREVIEW_CHARS: usize = 6_000;
/// Files above this size are not read for previews.
const MAX_PREVIEW_BYTES: u64 = 8 * 1024 * 1024;

/// Copies and inspects attachments.
pub struct AttachmentStager;

fn attachments_dir(workspace: &Path) -> PathBuf {
    workspace.join(RUNTIME_DIR).join("attachments")
}

/// Count PDF pages by scanning for page objects. Cheap, dependency-free and
/// good enough to tell the user how big a document is.
///
/// `/Type /Pages` is the page-tree node, not a page, so those matches are
/// subtracted from the raw `/Type /Page` count.
pub fn pdf_page_hint(bytes: &[u8]) -> usize {
    let haystack = String::from_utf8_lossy(bytes);
    let pages = haystack.matches("/Type /Page").count() + haystack.matches("/Type/Page").count();
    let trees = haystack.matches("/Pages").count();
    pages.saturating_sub(trees)
}

impl AttachmentStager {
    /// Stage one file. `workspace` is where `.reverie/attachments` lives.
    pub async fn stage_one(source: &Path, workspace: &Path) -> Result<SessionAttachment, String> {
        let metadata = std::fs::metadata(source)
            .map_err(|err| format!("cannot read {}: {err}", source.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a regular file", source.display()));
        }

        let file_name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "attachment".to_string());
        let mime_type = util::guess_mime(&file_name);
        let kind = AttachmentKind::classify(&mime_type, &file_name);
        let size = metadata.len();

        let dir = attachments_dir(workspace);
        std::fs::create_dir_all(&dir).map_err(|err| format!("cannot create staging dir: {err}"))?;

        let mut attachment = SessionAttachment::staged(
            file_name.clone(),
            source.to_string_lossy().to_string(),
            mime_type.clone(),
            size as i64,
        );

        // Copy into the workspace so the session stays self-contained.
        let prefix = util::char_prefix(&attachment.id, 8).to_string();
        let target = dir.join(format!("{prefix}-{}", sanitize(&file_name)));
        match std::fs::copy(source, &target) {
            Ok(_) => attachment.file_path = target.to_string_lossy().to_string(),
            Err(_) => {
                // Read-only sources still work: keep the original path.
                attachment.file_path = source.to_string_lossy().to_string();
            }
        }

        if kind.is_vision_capable() && size <= MAX_THUMBNAIL_BYTES {
            if let Ok(bytes) = std::fs::read(&attachment.file_path) {
                attachment.base64_thumbnail = Some(util::encode_base64(&bytes));
            }
        }

        attachment.extracted_text_preview = match kind {
            AttachmentKind::Text | AttachmentKind::Data => {
                if size <= MAX_PREVIEW_BYTES {
                    std::fs::read_to_string(&attachment.file_path)
                        .ok()
                        .map(|text| util::truncate_middle(&text, PREVIEW_CHARS))
                } else {
                    Some(format!("(file is {} — too large to preview inline)", util::human_bytes(size)))
                }
            }
            AttachmentKind::Pdf => std::fs::read(&attachment.file_path).ok().map(|bytes| {
                let pages = pdf_page_hint(&bytes);
                format!(
                    "PDF document, {pages} page(s), {}.\n\
                     This preview carries no extracted text: read it with a vision-capable model via \
                     `inspect_attachment` mode=vision, or convert it to text first.",
                    util::human_bytes(size)
                )
            }),
            AttachmentKind::Image => Some(format!(
                "Image attachment ({mime_type}, {}). Use `inspect_attachment` mode=vision to look at it.",
                util::human_bytes(size)
            )),
            AttachmentKind::Binary => None,
        };

        Ok(attachment)
    }

    /// Stage a batch, returning both successes and per-file failures.
    pub async fn stage_batch(
        paths: &[String],
        workspace: &Path,
    ) -> (Vec<SessionAttachment>, Vec<String>) {
        let mut staged = Vec::new();
        let mut errors = Vec::new();
        for raw in paths {
            match Self::stage_one(Path::new(raw), workspace).await {
                Ok(attachment) => staged.push(attachment),
                Err(err) => errors.push(err),
            }
        }
        (staged, errors)
    }

    /// Stage raw bytes (clipboard paste, drag-and-drop from a browser).
    pub fn stage_bytes(
        file_name: &str,
        mime_type: &str,
        bytes: &[u8],
        workspace: &Path,
    ) -> Result<SessionAttachment, String> {
        if bytes.is_empty() {
            return Err("the pasted payload is empty".to_string());
        }
        let dir = attachments_dir(workspace);
        std::fs::create_dir_all(&dir).map_err(|err| format!("cannot create staging dir: {err}"))?;

        let kind = AttachmentKind::classify(mime_type, file_name);
        let mut attachment = SessionAttachment::staged(
            file_name,
            String::new(),
            mime_type,
            bytes.len() as i64,
        );

        let target = dir.join(format!("{}-{}", util::char_prefix(&attachment.id, 8), sanitize(file_name)));
        std::fs::write(&target, bytes).map_err(|err| format!("cannot write staged file: {err}"))?;
        attachment.file_path = target.to_string_lossy().to_string();

        if kind.is_vision_capable() && bytes.len() as u64 <= MAX_THUMBNAIL_BYTES {
            attachment.base64_thumbnail = Some(util::encode_base64(bytes));
        }

        attachment.extracted_text_preview = match kind {
            AttachmentKind::Text | AttachmentKind::Data => {
                String::from_utf8(bytes.to_vec()).ok().map(|text| util::truncate_middle(&text, PREVIEW_CHARS))
            }
            AttachmentKind::Pdf => {
                let pages = pdf_page_hint(bytes);
                Some(format!(
                    "PDF document, {pages} page(s), {}.\nRead it with a vision-capable model via \
                     `inspect_attachment` mode=vision.",
                    util::human_bytes(bytes.len() as u64)
                ))
            }
            AttachmentKind::Image => Some(format!(
                "Image attachment ({mime_type}). Use `inspect_attachment` mode=vision to look at it."
            )),
            AttachmentKind::Binary => None,
        };

        Ok(attachment)
    }
}

/// Keep only characters that are safe in a file name.
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' { ch } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reverie-stage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn text_files_get_a_preview_and_are_copied() {
        let dir = temp_dir();
        let source = dir.join("notes.md");
        std::fs::write(&source, "# Notes\nimportant content").unwrap();

        let workspace = dir.join("ws");
        std::fs::create_dir_all(&workspace).unwrap();

        let staged = AttachmentStager::stage_one(&source, &workspace).await.unwrap();
        assert_eq!(staged.mime_type, "text/markdown");
        assert!(staged.extracted_text_preview.unwrap().contains("important content"));
        assert!(staged.file_path.contains(".reverie"));
        assert!(Path::new(&staged.file_path).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn images_get_base64_previews() {
        let dir = temp_dir();
        let source = dir.join("shot.png");
        let bytes = util::decode_base64(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
        )
        .unwrap();
        std::fs::write(&source, bytes).unwrap();

        let staged = AttachmentStager::stage_one(&source, &dir).await.unwrap();
        assert!(staged.is_image());
        assert!(staged.base64_thumbnail.is_some());
        assert!(staged.extracted_text_preview.unwrap().contains("mode=vision"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn batch_reports_failures_without_aborting() {
        let dir = temp_dir();
        let good = dir.join("a.txt");
        std::fs::write(&good, "hello").unwrap();
        let missing = dir.join("missing.txt");

        let (staged, errors) = AttachmentStager::stage_batch(
            &[good.to_string_lossy().to_string(), missing.to_string_lossy().to_string()],
            &dir,
        )
        .await;
        assert_eq!(staged.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("cannot read"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pdf_page_hint_counts_page_objects() {
        let bytes = b"%PDF-1.4\n/Type /Page\n/Type/Page\n/Type /Pages";
        assert_eq!(pdf_page_hint(bytes), 2);
    }

    #[test]
    fn sanitize_strips_separators() {
        assert_eq!(sanitize("a/b:c.txt"), "a_b_c.txt");
    }
}
