//! Unified LLM protocol types: messages, content parts, thinking depth and
//! streaming chunk shape. Every provider adapter translates to and from these.

pub mod message;
pub mod request;
pub mod thinking;

pub use message::{ChatMessage, ContentPart, MessageRole, ToolCall, ToolSpec};
pub use request::{FinishReason, LlmRequest, RemoteModel, StreamChunk, TokenUsage};
pub use thinking::ThinkingLevel;
