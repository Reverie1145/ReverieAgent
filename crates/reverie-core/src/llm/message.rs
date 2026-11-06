//! Chat messages and multimodal content parts.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};

/// Conversation role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        }
    }
}

/// A single piece of message content.
///
/// `Reasoning` blocks are persisted alongside the final answer so the UI can
/// replay a thinking trace without re-running the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    Text { text: String },
    Reasoning { text: String },
    ImageBase64 { mime_type: String, data: String },
    FileDocument { file_name: String, mime_type: String, content: String },
}

impl ContentPart {
    pub fn text(value: impl Into<String>) -> Self {
        ContentPart::Text { text: value.into() }
    }

    pub fn reasoning(value: impl Into<String>) -> Self {
        ContentPart::Reasoning { text: value.into() }
    }

    pub fn image(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        ContentPart::ImageBase64 { mime_type: mime_type.into(), data: data.into() }
    }

    pub fn document(file_name: impl Into<String>, mime_type: impl Into<String>, content: impl Into<String>) -> Self {
        ContentPart::FileDocument {
            file_name: file_name.into(),
            mime_type: mime_type.into(),
            content: content.into(),
        }
    }

    /// Whether this part can be flattened to plain text.
    pub fn is_textual(&self) -> bool {
        matches!(self, ContentPart::Text { .. } | ContentPart::Reasoning { .. } | ContentPart::FileDocument { .. })
    }

    /// Render the part as text, if it has a textual representation.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text { text } | ContentPart::Reasoning { text } => Some(text),
            ContentPart::FileDocument { content, .. } => Some(content),
            ContentPart::ImageBase64 { .. } => None,
        }
    }
}

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Parse the raw argument string emitted by a provider, tolerating the
    /// empty-string case that several vendors send for zero-arg tools.
    pub fn parse_arguments(raw: &str) -> serde_json::Value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return serde_json::Value::Object(serde_json::Map::new());
        }
        serde_json::from_str(trimmed).unwrap_or_else(|_| serde_json::json!({ "_raw": trimmed }))
    }
}

/// Tool declaration handed to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: serde_json::Value) -> Self {
        ToolSpec { name: name.into(), description: description.into(), parameters }
    }
}

/// One entry in an agent's transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    #[serde(default)]
    pub parts: Vec<ContentPart>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(text: impl Into<String>) -> Self {
        Self::text(MessageRole::System, text)
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::text(MessageRole::User, text)
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::text(MessageRole::Assistant, text)
    }

    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        ChatMessage { role, parts: vec![ContentPart::text(text)], tool_calls: Vec::new(), tool_call_id: None, name: None }
    }

    pub fn with_parts(role: MessageRole, parts: Vec<ContentPart>) -> Self {
        ChatMessage { role, parts, tool_calls: Vec::new(), tool_call_id: None, name: None }
    }

    pub fn tool_result(call_id: impl Into<String>, tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: MessageRole::Tool,
            parts: vec![ContentPart::text(content)],
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            name: Some(tool_name.into()),
        }
    }

    pub fn with_tool_calls(mut self, calls: Vec<ToolCall>) -> Self {
        self.tool_calls = calls;
        self
    }

    /// Concatenate every textual part (excluding reasoning).
    pub fn text_content(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                ContentPart::Text { text } | ContentPart::FileDocument { content: text, .. } => {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
                ContentPart::Reasoning { .. } | ContentPart::ImageBase64 { .. } => {}
            }
        }
        out
    }

    /// Concatenate every reasoning part.
    pub fn reasoning_content(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            if let ContentPart::Reasoning { text } = part {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
        out
    }

    /// All inline images carried by this message.
    pub fn images(&self) -> Vec<(&str, &str)> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                ContentPart::ImageBase64 { mime_type, data } => Some((mime_type.as_str(), data.as_str())),
                _ => None,
            })
            .collect()
    }

    pub fn has_images(&self) -> bool {
        self.parts.iter().any(|part| matches!(part, ContentPart::ImageBase64 { .. }))
    }

    /// Encode a transcript for the `message_history_json` column.
    pub fn encode_history(messages: &[ChatMessage]) -> CoreResult<String> {
        serde_json::to_string(messages).map_err(|err| CoreError::Serialization(err.to_string()))
    }

    /// Decode a transcript, returning an empty vector for legacy/corrupt rows
    /// rather than failing the whole agent load.
    pub fn decode_history(raw: &str) -> Vec<ChatMessage> {
        serde_json::from_str(raw).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_reasoning_are_separated() {
        let msg = ChatMessage::with_parts(
            MessageRole::Assistant,
            vec![ContentPart::reasoning("thinking…"), ContentPart::text("answer")],
        );
        assert_eq!(msg.text_content(), "answer");
        assert_eq!(msg.reasoning_content(), "thinking…");
    }

    #[test]
    fn history_roundtrip_preserves_images() {
        let msg = ChatMessage::with_parts(
            MessageRole::User,
            vec![ContentPart::text("look"), ContentPart::image("image/png", "AAAA")],
        );
        let encoded = ChatMessage::encode_history(&[msg.clone()]).unwrap();
        let decoded = ChatMessage::decode_history(&encoded);
        assert_eq!(decoded.len(), 1);
        assert!(decoded[0].has_images());
        assert_eq!(decoded[0].images()[0].0, "image/png");
    }

    #[test]
    fn corrupt_history_degrades_to_empty() {
        assert!(ChatMessage::decode_history("<<not json>>").is_empty());
    }

    #[test]
    fn empty_tool_arguments_parse_to_object() {
        assert_eq!(ToolCall::parse_arguments(""), serde_json::json!({}));
        assert_eq!(ToolCall::parse_arguments("{\"a\":1}"), serde_json::json!({"a":1}));
        assert!(ToolCall::parse_arguments("{broken").get("_raw").is_some());
    }
}
