//! Protocol adapters. Each adapter owns request shaping, thinking-depth
//! translation, multimodal encoding and stream decoding for one wire protocol.

pub mod anthropic;
pub mod gemini;
pub mod ollama;
pub mod openai;

use async_trait::async_trait;
use futures::stream::BoxStream;
use reverie_core::domain::{Provider, Protocol};
use reverie_core::llm::{ContentPart, LlmRequest, RemoteModel, StreamChunk};

use crate::error::LlmResult;

/// Uniform interface implemented by every protocol adapter.
#[async_trait]
pub trait LlmAdapter: Send + Sync {
    fn protocol(&self) -> Protocol;

    /// Start a streaming completion.
    async fn stream_chat(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<BoxStream<'static, LlmResult<StreamChunk>>>;

    /// Discover the provider's live model list.
    async fn list_models(&self, provider: &Provider) -> LlmResult<Vec<RemoteModel>>;
}

/// Join a base URL with an API path without producing double slashes.
pub fn join_url(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let path = path.trim().trim_start_matches('/');
    format!("{base}/{path}")
}

/// Standard `Authorization: Bearer` header value.
pub fn bearer(provider: &Provider) -> String {
    format!("Bearer {}", provider.api_key.trim())
}

/// Render a document attachment as a fenced context block so text-only models
/// can still reason about it.
pub fn document_block(file_name: &str, mime_type: &str, content: &str) -> String {
    let fence = if content.contains("```") { "````" } else { "```" };
    format!("### Attached file: {file_name} ({mime_type})\n{fence}\n{content}\n{fence}")
}

/// Flatten content parts into a single text block, describing images inline.
/// Used by protocols that cannot accept binary image blocks in a given position.
pub fn flatten_parts(parts: &[ContentPart]) -> String {
    let mut out = String::new();
    for part in parts {
        let rendered = match part {
            ContentPart::Text { text } => text.clone(),
            ContentPart::Reasoning { .. } => continue,
            ContentPart::ImageBase64 { mime_type, .. } => format!("[attached image: {mime_type}]"),
            ContentPart::FileDocument { file_name, mime_type, content } => {
                document_block(file_name, mime_type, content)
            }
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&rendered);
    }
    out
}

/// True when any message in the request carries an image.
pub fn request_has_images(request: &LlmRequest) -> bool {
    request.messages.iter().any(|message| message.has_images())
}

/// A data URL for OpenAI-style inline images.
pub fn data_url(mime_type: &str, data: &str) -> String {
    format!("data:{mime_type};base64,{data}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::llm::ChatMessage;

    #[test]
    fn url_joining_is_clean() {
        assert_eq!(join_url("https://x/v1/", "/chat"), "https://x/v1/chat");
        assert_eq!(join_url("https://x/v1", "chat"), "https://x/v1/chat");
    }

    #[test]
    fn documents_are_fenced() {
        let block = document_block("a.rs", "text/plain", "fn main() {}");
        assert!(block.starts_with("### Attached file: a.rs"));
        assert!(block.contains("```"));
    }

    #[test]
    fn documents_with_fences_use_longer_fence() {
        let block = document_block("a.md", "text/markdown", "```rust\nx\n```");
        assert!(block.contains("````"));
    }

    #[test]
    fn flattening_describes_images() {
        let parts = vec![
            ContentPart::text("look"),
            ContentPart::image("image/png", "AAAA"),
            ContentPart::reasoning("hidden"),
        ];
        let flat = flatten_parts(&parts);
        assert!(flat.contains("look"));
        assert!(flat.contains("[attached image: image/png]"));
        assert!(!flat.contains("hidden"));
    }

    #[test]
    fn image_detection_scans_messages() {
        let mut req = LlmRequest::new("m", vec![ChatMessage::user("hi")]);
        assert!(!request_has_images(&req));
        req.messages.push(ChatMessage::with_parts(
            reverie_core::llm::MessageRole::User,
            vec![ContentPart::image("image/png", "x")],
        ));
        assert!(request_has_images(&req));
    }

    #[test]
    fn data_url_shape() {
        assert_eq!(data_url("image/png", "AAA"), "data:image/png;base64,AAA");
    }
}
