//! Request/response envelopes shared by every protocol adapter.

use serde::{Deserialize, Serialize};

use super::message::{ChatMessage, ToolSpec};
use super::thinking::ThinkingLevel;

/// A normalized chat completion request.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub thinking: ThinkingLevel,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f64>,
    /// Ask the provider to stream. Adapters always stream internally; this flag
    /// is forwarded so providers that behave differently can honor it.
    pub stream: bool,
}

impl LlmRequest {
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        LlmRequest {
            model: model.into(),
            messages,
            tools: Vec::new(),
            thinking: ThinkingLevel::default(),
            max_tokens: None,
            temperature: None,
            stream: true,
        }
    }

    pub fn with_tools(mut self, tools: Vec<ToolSpec>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_thinking(mut self, thinking: ThinkingLevel) -> Self {
        self.thinking = thinking;
        self
    }

    pub fn with_limits(mut self, max_tokens: Option<u32>, temperature: Option<f64>) -> Self {
        self.max_tokens = max_tokens;
        self.temperature = temperature;
        self
    }

    /// Split the leading system messages from the conversational remainder.
    /// Anthropic and Gemini take the system prompt out-of-band.
    pub fn split_system(&self) -> (Option<String>, Vec<&ChatMessage>) {
        let mut system = String::new();
        let mut rest = Vec::new();
        for msg in &self.messages {
            if matches!(msg.role, super::message::MessageRole::System) {
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(&msg.text_content());
            } else {
                rest.push(msg);
            }
        }
        let system = if system.trim().is_empty() { None } else { Some(system) };
        (system, rest)
    }
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
}

impl FinishReason {
    pub fn from_wire(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "stop" | "end_turn" | "stop_sequence" | "complete" => FinishReason::Stop,
            "length" | "max_tokens" | "max_output_tokens" => FinishReason::Length,
            "tool_calls" | "tool_use" | "function_call" => FinishReason::ToolCalls,
            "content_filter" | "safety" | "recitation" => FinishReason::ContentFilter,
            _ => FinishReason::Stop,
        }
    }
}

/// Token accounting reported by the provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub reasoning_tokens: u32,
}

impl TokenUsage {
    pub fn merge(&mut self, other: TokenUsage) {
        self.prompt_tokens = self.prompt_tokens.max(other.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(other.completion_tokens);
        self.reasoning_tokens = self.reasoning_tokens.max(other.reasoning_tokens);
    }

    pub fn total(&self) -> u32 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}

/// One increment of a streaming completion.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamChunk {
    /// Visible answer text.
    TextDelta(String),
    /// Reasoning / thinking text, surfaced separately in the UI.
    ReasoningDelta(String),
    /// Incremental tool-call construction.
    ToolCallDelta { index: usize, id: Option<String>, name: Option<String>, arguments_delta: String },
    /// Token accounting, usually emitted last.
    Usage(TokenUsage),
    /// Terminal marker.
    Done { finish_reason: FinishReason },
}

/// A model discovered by pulling a provider's live model list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModel {
    pub model_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub context_window: Option<i64>,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub owned_by: Option<String>,
    /// Provider-reported capability hints (Ollama `show`, Gemini metadata, …).
    #[serde(default)]
    pub hints: serde_json::Value,
}

impl RemoteModel {
    pub fn new(model_name: impl Into<String>) -> Self {
        RemoteModel {
            model_name: model_name.into(),
            display_name: None,
            context_window: None,
            family: None,
            owned_by: None,
            hints: serde_json::Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::message::MessageRole;

    #[test]
    fn system_messages_are_hoisted() {
        let req = LlmRequest::new(
            "m",
            vec![ChatMessage::system("be nice"), ChatMessage::user("hi"), ChatMessage::system("and brief")],
        );
        let (system, rest) = req.split_system();
        assert_eq!(system.as_deref(), Some("be nice\n\nand brief"));
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].role, MessageRole::User);
    }

    #[test]
    fn finish_reason_maps_provider_vocabulary() {
        assert_eq!(FinishReason::from_wire("end_turn"), FinishReason::Stop);
        assert_eq!(FinishReason::from_wire("max_tokens"), FinishReason::Length);
        assert_eq!(FinishReason::from_wire("tool_use"), FinishReason::ToolCalls);
        assert_eq!(FinishReason::from_wire("something_new"), FinishReason::Stop);
    }

    #[test]
    fn usage_merge_keeps_maximums() {
        let mut a = TokenUsage { prompt_tokens: 10, completion_tokens: 5, reasoning_tokens: 1 };
        a.merge(TokenUsage { prompt_tokens: 10, completion_tokens: 9, reasoning_tokens: 0 });
        assert_eq!(a.completion_tokens, 9);
        assert_eq!(a.reasoning_tokens, 1);
        assert_eq!(a.total(), 19);
    }
}
