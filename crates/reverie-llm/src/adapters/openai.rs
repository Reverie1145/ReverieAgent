//! OpenAI-compatible adapter (`/chat/completions`).
//!
//! Covers OpenAI itself plus DeepSeek, Groq, Together, OpenRouter, Moonshot,
//! Qwen, vLLM, LM Studio and any other server that speaks the same shape.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use reverie_core::domain::{Provider, Protocol};
use reverie_core::llm::{
    ChatMessage, ContentPart, FinishReason, LlmRequest, MessageRole, RemoteModel, StreamChunk, TokenUsage,
};
use serde_json::{json, Value};

use super::{bearer, data_url, document_block, flatten_parts, join_url, LlmAdapter};
use crate::error::{ensure_success, LlmError, LlmResult};
use crate::stream_util::{channel_pair, forward, sse_stream};

/// Adapter for the OpenAI chat-completions protocol.
pub struct OpenAiAdapter {
    client: reqwest::Client,
}

impl OpenAiAdapter {
    pub fn new(client: reqwest::Client) -> Self {
        OpenAiAdapter { client }
    }
}

/// Reasoning models reject `temperature` and require `max_completion_tokens`.
pub fn looks_like_reasoning_model(model: &str) -> bool {
    let name = model.to_ascii_lowercase();
    name.starts_with("o1")
        || name.starts_with("o3")
        || name.starts_with("o4")
        || name.contains("gpt-5")
        || name.contains("reasoner")
        || name.contains("reasoning")
        || name.contains("deepseek-r1")
        || name.contains("qwq")
        || name.contains("thinking")
}

fn user_content(message: &ChatMessage) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => {
                if !text.is_empty() {
                    parts.push(json!({"type": "text", "text": text}));
                }
            }
            ContentPart::ImageBase64 { mime_type, data } => {
                parts.push(json!({
                    "type": "image_url",
                    "image_url": {"url": data_url(mime_type, data)}
                }));
            }
            ContentPart::FileDocument { file_name, mime_type, content } => {
                parts.push(json!({
                    "type": "text",
                    "text": document_block(file_name, mime_type, content)
                }));
            }
            ContentPart::Reasoning { .. } => {}
        }
    }
    if parts.is_empty() {
        Value::String(String::new())
    } else {
        Value::Array(parts)
    }
}

fn message_to_json(message: &ChatMessage) -> Value {
    match message.role {
        MessageRole::System => json!({"role": "system", "content": message.text_content()}),
        MessageRole::User => {
            if message.has_images() {
                json!({"role": "user", "content": user_content(message)})
            } else {
                json!({"role": "user", "content": flatten_parts(&message.parts)})
            }
        }
        MessageRole::Assistant => {
            let text = message.text_content();
            let mut object = serde_json::Map::new();
            object.insert("role".to_string(), json!("assistant"));
            object.insert(
                "content".to_string(),
                if text.is_empty() { Value::Null } else { Value::String(text) },
            );
            if !message.tool_calls.is_empty() {
                let calls: Vec<Value> = message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                object.insert("tool_calls".to_string(), Value::Array(calls));
            }
            Value::Object(object)
        }
        MessageRole::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id.clone().unwrap_or_default(),
            "content": message.text_content(),
        }),
    }
}

/// Shape the request body. Kept pure so it can be unit tested.
pub fn build_body(request: &LlmRequest) -> Value {
    let reasoning_model = looks_like_reasoning_model(&request.model);

    let mut body = json!({
        "model": request.model,
        "messages": request.messages.iter().map(message_to_json).collect::<Vec<_>>(),
        "stream": true,
        "stream_options": {"include_usage": true},
    });

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = json!("auto");
    }

    if let Some(effort) = request.thinking.openai_reasoning_effort() {
        body["reasoning_effort"] = json!(effort);
    }

    if let Some(max_tokens) = request.max_tokens {
        if reasoning_model {
            body["max_completion_tokens"] = json!(max_tokens);
        } else {
            body["max_tokens"] = json!(max_tokens);
        }
    }

    if let Some(temperature) = request.temperature {
        if !reasoning_model {
            body["temperature"] = json!(temperature);
        }
    }

    body
}

async fn pump(tx: tokio::sync::mpsc::Sender<LlmResult<StreamChunk>>, response: reqwest::Response) -> LlmResult<()> {
    let mut events = sse_stream(response);
    let mut finished = false;

    while let Some(event) = events.next().await {
        let event = event?;
        let data = event.data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            break;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            // Some gateways emit keep-alive comments as data frames.
            continue;
        };

        if let Some(error) = chunk.get("error").filter(|value| !value.is_null()) {
            return Err(LlmError::Protocol(format!("provider stream error: {error}")));
        }

        if let Some(usage) = chunk.get("usage").filter(|value| !value.is_null()) {
            let tokens = TokenUsage {
                prompt_tokens: usage.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                completion_tokens: usage.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
                reasoning_tokens: usage
                    .get("completion_tokens_details")
                    .and_then(|details| details.get("reasoning_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32,
            };
            let _ = tx.send(Ok(StreamChunk::Usage(tokens))).await;
        }

        let Some(choice) = chunk.get("choices").and_then(|choices| choices.get(0)) else {
            continue;
        };
        let delta = &choice["delta"];

        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            if !text.is_empty() && !forward(&tx, StreamChunk::TextDelta(text.to_string())).await {
                return Ok(());
            }
        }

        let reasoning = delta
            .get("reasoning_content")
            .and_then(Value::as_str)
            .or_else(|| delta.get("reasoning").and_then(Value::as_str));
        if let Some(text) = reasoning {
            if !text.is_empty() && !forward(&tx, StreamChunk::ReasoningDelta(text.to_string())).await {
                return Ok(());
            }
        }

        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for (position, call) in calls.iter().enumerate() {
                let index = call
                    .get("index")
                    .and_then(Value::as_u64)
                    .map(|value| value as usize)
                    .unwrap_or(position);
                let id = call.get("id").and_then(Value::as_str).map(str::to_string);
                let name = call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let arguments = call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let delta_chunk = StreamChunk::ToolCallDelta { index, id, name, arguments_delta: arguments };
                if !forward(&tx, delta_chunk).await {
                    return Ok(());
                }
            }
        }

        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            if !reason.is_empty() {
                finished = true;
                let _ = tx
                    .send(Ok(StreamChunk::Done { finish_reason: FinishReason::from_wire(reason) }))
                    .await;
            }
        }
    }

    if !finished {
        let _ = tx.send(Ok(StreamChunk::Done { finish_reason: FinishReason::Stop })).await;
    }
    Ok(())
}

#[async_trait]
impl LlmAdapter for OpenAiAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::OpenAiCompat
    }

    async fn stream_chat(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<BoxStream<'static, LlmResult<StreamChunk>>> {
        let url = join_url(&provider.normalized_base_url(), "/chat/completions");
        let body = build_body(&request);

        let response = self
            .client
            .post(&url)
            .header("Authorization", bearer(provider))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        let response = ensure_success(response).await?;

        let (tx, stream) = channel_pair();
        tokio::spawn(async move {
            if let Err(err) = pump(tx.clone(), response).await {
                let _ = tx.send(Err(err)).await;
            }
        });
        Ok(stream)
    }

    async fn list_models(&self, provider: &Provider) -> LlmResult<Vec<RemoteModel>> {
        let url = join_url(&provider.normalized_base_url(), "/models");
        let response = self
            .client
            .get(&url)
            .header("Authorization", bearer(provider))
            .send()
            .await?;
        let response = ensure_success(response).await?;
        let payload: Value = response.json().await?;

        let items = payload
            .get("data")
            .and_then(Value::as_array)
            .or_else(|| payload.get("models").and_then(Value::as_array))
            .cloned()
            .unwrap_or_default();

        Ok(items
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_str)?.trim().to_string();
                if id.is_empty() {
                    return None;
                }
                let mut model = RemoteModel::new(id);
                model.owned_by = item.get("owned_by").and_then(Value::as_str).map(str::to_string);
                model.display_name = item.get("name").and_then(Value::as_str).map(str::to_string);
                model.context_window = item
                    .get("context_length")
                    .and_then(Value::as_i64)
                    .or_else(|| item.get("context_window").and_then(Value::as_i64))
                    .or_else(|| item.get("max_context_length").and_then(Value::as_i64));
                model.hints = item.clone();
                Some(model)
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::llm::{ThinkingLevel, ToolSpec};

    fn request() -> LlmRequest {
        LlmRequest::new("gpt-4o-mini", vec![ChatMessage::system("sys"), ChatMessage::user("hi")])
    }

    #[test]
    fn body_carries_stream_and_usage_options() {
        let body = build_body(&request());
        assert_eq!(body["model"], "gpt-4o-mini");
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hi");
    }

    #[test]
    fn reasoning_effort_is_translated() {
        let body = build_body(&request().with_thinking(ThinkingLevel::High));
        assert_eq!(body["reasoning_effort"], "high");
        let off = build_body(&request().with_thinking(ThinkingLevel::Off));
        assert!(off.get("reasoning_effort").is_none());
    }

    #[test]
    fn reasoning_models_get_completion_tokens() {
        let mut req = request();
        req.model = "o3-mini".into();
        let body = build_body(&req.with_limits(Some(512), Some(0.7)));
        assert_eq!(body["max_completion_tokens"], 512);
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn tools_are_serialized() {
        let tool = ToolSpec::new("web_search", "search", json!({"type":"object"}));
        let body = build_body(&request().with_tools(vec![tool]));
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "web_search");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn images_become_data_urls() {
        let mut req = request();
        req.messages.push(ChatMessage::with_parts(
            MessageRole::User,
            vec![ContentPart::text("describe"), ContentPart::image("image/png", "QUJD")],
        ));
        let body = build_body(&req);
        let parts = body["messages"][2]["content"].as_array().unwrap();
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

    #[test]
    fn assistant_tool_calls_roundtrip() {
        let mut req = request();
        req.messages.push(
            ChatMessage::assistant("").with_tool_calls(vec![reverie_core::llm::ToolCall {
                id: "call_1".into(),
                name: "fs_read_file".into(),
                arguments: json!({"path": "a.rs"}),
            }]),
        );
        let body = build_body(&req);
        let assistant = &body["messages"][2];
        assert!(assistant["content"].is_null());
        assert_eq!(assistant["tool_calls"][0]["function"]["name"], "fs_read_file");
    }

    #[test]
    fn tool_messages_reference_call_id() {
        let mut req = request();
        req.messages.push(ChatMessage::tool_result("call_9", "fs_read_file", "content"));
        let body = build_body(&req);
        assert_eq!(body["messages"][2]["role"], "tool");
        assert_eq!(body["messages"][2]["tool_call_id"], "call_9");
    }

    #[test]
    fn reasoning_model_detection() {
        assert!(looks_like_reasoning_model("o3-mini"));
        assert!(looks_like_reasoning_model("deepseek-r1-distill"));
        assert!(looks_like_reasoning_model("gpt-5-turbo"));
        assert!(!looks_like_reasoning_model("gpt-4o-mini"));
    }
}
