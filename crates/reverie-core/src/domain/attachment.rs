//! Multimodal attachments staged for a session.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time;

pub use crate::util::AttachmentKind;

/// A file or image the user attached to a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAttachment {
    pub id: String,
    pub session_id: String,
    pub file_name: String,
    pub file_path: String,
    pub mime_type: String,
    pub file_size_bytes: i64,
    pub base64_thumbnail: Option<String>,
    pub extracted_text_preview: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl SessionAttachment {
    pub fn create(
        session_id: impl Into<String>,
        file_name: impl Into<String>,
        file_path: impl Into<String>,
        mime_type: impl Into<String>,
        file_size_bytes: i64,
    ) -> Self {
        SessionAttachment {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.into(),
            file_name: file_name.into(),
            file_path: file_path.into(),
            mime_type: mime_type.into(),
            file_size_bytes,
            base64_thumbnail: None,
            extracted_text_preview: None,
            created_at: time::now(),
        }
    }

    /// Staged but not yet bound to a persisted session (session_id is empty).
    pub fn staged(
        file_name: impl Into<String>,
        file_path: impl Into<String>,
        mime_type: impl Into<String>,
        file_size_bytes: i64,
    ) -> Self {
        Self::create(String::new(), file_name, file_path, mime_type, file_size_bytes)
    }

    pub fn kind(&self) -> AttachmentKind {
        AttachmentKind::classify(&self.mime_type, &self.file_name)
    }

    pub fn is_image(&self) -> bool {
        self.kind().is_vision_capable()
    }

    pub fn with_thumbnail(mut self, thumbnail: impl Into<String>) -> Self {
        self.base64_thumbnail = Some(thumbnail.into());
        self
    }

    pub fn with_text_preview(mut self, preview: impl Into<String>) -> Self {
        self.extracted_text_preview = Some(preview.into());
        self
    }

    /// One-line summary for a context block.
    pub fn describe(&self) -> String {
        format!(
            "{} ({}, {})",
            self.file_name,
            self.mime_type,
            crate::util::human_bytes(self.file_size_bytes.max(0) as u64)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_detected_from_mime() {
        let a = SessionAttachment::staged("shot.png", "/tmp/shot.png", "image/png", 1024);
        assert!(a.is_image());
        assert_eq!(a.kind(), AttachmentKind::Image);
    }

    #[test]
    fn code_files_are_textual() {
        let a = SessionAttachment::staged("lib.rs", "/tmp/lib.rs", "application/octet-stream", 2048);
        assert!(!a.is_image());
        assert_eq!(a.kind(), AttachmentKind::Text);
    }

    #[test]
    fn describe_includes_human_size() {
        let a = SessionAttachment::staged("a.csv", "/tmp/a.csv", "text/csv", 2048);
        assert!(a.describe().contains("2.0 KB"));
        assert!(a.describe().contains("text/csv"));
    }
}
