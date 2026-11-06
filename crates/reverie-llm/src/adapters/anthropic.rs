//! Anthropic Messages API adapter, including extended-thinking budgets.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use reverie_core::domain::{Provider, Protocol};
use reverie_core::llm::{
    ChatMessage, ContentPart, FinishReason, LlmRequest, MessageRole, RemoteModel, StreamChunk, TokenUsage,
};
use serde_json::{json, Value};

use super::{document_block, join_url, LlmAdapter};
use crate::error::{ensure_success, LlmError, LlmResult};
use crate::stream_util::{channel_pair, forward, sse_stream};

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Adapter for `POST /messages`.
pub struct AnthropicAdapter {
    client: reqwest::Client,
}

impl AnthropicAdapter {
    pub fn new(client: reqwest::Client) -> Self {
        AnthropicAdapter { client }
    }

    fn headers(&self, builder: reqwest::RequestBuilder, provider: &Provider) -> reqwest::RequestBuilder {
        builder
            .header("x-api-key", provider.api_key.trim())
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
    }
}

/// Build the `content` array for a user/assistant turn.
fn content_blocks(message: &ChatMessage) -> Vec<Value> {
    let mut blocks: Vec<Value> = Vec::new();
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => {
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
            }
            ContentPart::ImageBase64 { mime_type, data } => {
                blocks.push(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": mime_type, "data": data}
                }));
            }
            ContentPart::FileDocument { file_name, mime_type, content } => {
                blocks.push(json!({"type": "text", "text": document_block(file_name, mime_type, content)}));
            }
            ContentPart::Reasoning { .. } => {}
        }
    }
    if !message.tool_calls.is_empty() {
        for call in &message.tool_calls {
            blocks.push(json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.arguments,
            }));
        }
    }
    blocks
}

/// Anthropic requires alternating roles and `tool_result` blocks inside a user
/// message, so adjacent same-role turns are merged.
pub fn build_messages(messages: &[&ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        let (role, blocks) = match message.role {
            MessageRole::Assistant => ("assistant", content_blocks(message)),
            MessageRole::Tool => (
                "user",
                vec![json!({
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.text_content(),
                })],
            ),
            _ => ("user", content_blocks(message)),
        };
        if blocks.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some(role) {
                if let Some(existing) = last.get_mut("content").and_then(Value::as_array_mut) {
                    existing.extend(blocks);
                    continue;
                }
            }
        }
        out.push(json!({"role": role, "content": blocks}));
    }
    out
}

/// Shape the request body. Pure, so it is unit tested without network access.
pub fn build_body(request: &LlmRequest) -> Value {
    let (system, rest) = request.split_system();
    let budget = request.thinking.anthropic_budget();
    let floor = budget.map(|value| value + 1024).unwrap_or(4096);
    let max_tokens = request.max_tokens.unwrap_or(4096).max(floor);

    let mut body = json!({
        "model": request.model,
        "max_tokens": max_tokens,
        "stream": true,
        "messages": build_messages(&rest),
    });

    if let Some(system) = system {
        body["system"] = json!(system);
    }

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect();
        body["tools"] = Value::Array(tools);
    }

    if let Some(budget) = budget {
        body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
    }

    if let Some(temperature) = request.temperature {
        if budget.is_none() {
            body["temperature"] = json!(temperature);
        }
    }

    body
}

async fn pump(tx: tokio::sync::mpsc::Sender<LlmResult<StreamChunk>>, response: reqwest::Response) -> LlmResult<()> {
    let mut events = sse_stream(response);
    let mut prompt_tokens = 0u32;
    let mut finished = false;

    while let Some(event) = events.next().await {
        let event = event?;
        let data = event.data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(data) else {
            continue;
        };

        match chunk.get("type").and_then(Value::as_str).unwrap_or_default() {
            "error" => {
                let message = chunk
                    .get("error")
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "unknown anthropic error".to_string());
                return Err(LlmError::Protocol(message));
            }
            "message_start" => {
                prompt_tokens = chunk["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
            }
            "content_block_start" => {
                let index = chunk.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let block = &chunk["content_block"];
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    let delta = StreamChunk::ToolCallDelta {
                        index,
                        id: block.get("id").and_then(Value::as_str).map(str::to_string),
                        name: block.get("name").and_then(Value::as_str).map(str::to_string),
                        arguments_delta: String::new(),
                    };
                    if !forward(&tx, delta).await {
                        return Ok(());
                    }
                }
            }
            "content_block_delta" => {
                let index = chunk.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                let delta = &chunk["delta"];
                match delta.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if !text.is_empty() && !forward(&tx, StreamChunk::TextDelta(text.to_string())).await {
                                return Ok(());
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            if !text.is_empty()
                                && !forward(&tx, StreamChunk::ReasoningDelta(text.to_string())).await
                            {
                                return Ok(());
                            }
                        }
                    }
                    "input_json_delta" => {
                        let partial =
                            delta.get("partial_json").and_then(Value::as_str).unwrap_or("").to_string();
                        let delta_chunk = StreamChunk::ToolCallDelta {
                            index,
                            id: None,
                            name: None,
                            arguments_delta: partial,
                        };
                        if !forward(&tx, delta_chunk).await {
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                let completion_tokens = chunk["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
                let _ = tx
                    .send(Ok(StreamChunk::Usage(TokenUsage {
                        prompt_tokens,
                        completion_tokens,
                        reasoning_tokens: 0,
                    })))
                    .await;
                if let Some(reason) = chunk["delta"]["stop_reason"].as_str() {
                    finished = true;
                    let _ = tx
                        .send(Ok(StreamChunk::Done { finish_reason: FinishReason::from_wire(reason) }))
                        .await;
                }
            }
            "message_stop" => {
                if !finished {
                    finished = true;
                    let _ = tx
                        .send(Ok(StreamChunk::Done { finish_reason: FinishReason::Stop }))
                        .await;
                }
            }
            _ => {}
        }
    }

    if !finished {
        let _ = tx.send(Ok(StreamChunk::Done { finish_reason: FinishReason::Stop })).await;
    }
    Ok(())
}

#[async_trait]
impl LlmAdapter for AnthropicAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Anthropic
    }

    async fn stream_chat(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<BoxStream<'static, LlmResult<StreamChunk>>> {
        let url = join_url(&provider.normalized_base_url(), "/messages");
        let body = build_body(&request);
        let builder = self.client.post(&url).json(&body);
        let response = self.headers(builder, provider).send().await?;
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
        let url = join_url(&provider.normalized_base_url(), "/models?limit=1000");
        let builder = self.client.get(&url);
        let response = self.headers(builder, provider).send().await?;
        let response = ensure_success(response).await?;
        let payload: Value = response.json().await?;

        let items = payload.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_str)?.trim().to_string();
                if id.is_empty() {
                    return None;
                }
                let mut model = RemoteModel::new(id);
                model.display_name = item.get("display_name").and_then(Value::as_str).map(str::to_string);
                model.owned_by = Some("anthropic".to_string());
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
        LlmRequest::new("claude-sonnet-4", vec![ChatMessage::system("be terse"), ChatMessage::user("hi")])
    }

    #[test]
    fn system_prompt_is_hoisted() {
        let body = build_body(&request());
        assert_eq!(body["system"], "be terse");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn thinking_budget_is_applied_and_max_tokens_grows() {
        let body = build_body(&request().with_thinking(ThinkingLevel::Medium));
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8192);
        assert!(body["max_tokens"].as_u64().unwrap() >= 8192 + 1024);
    }

    #[test]
    fn thinking_off_omits_block() {
        let body = build_body(&request().with_thinking(ThinkingLevel::Off));
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn images_use_base64_source() {
        let mut req = request();
        req.messages.push(ChatMessage::with_parts(
            MessageRole::User,
            vec![ContentPart::image("image/jpeg", "QUJD")],
        ));
        let body = build_body(&req);
        // The image turn is merged into the preceding user turn.
        let blocks = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["media_type"], "image/jpeg");
        assert_eq!(blocks[1]["source"]["data"], "QUJD");
    }

    #[test]
    fn adjacent_user_turns_are_merged() {
        let mut req = request();
        req.messages.push(ChatMessage::user("second"));
        let body = build_body(&req);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn tool_results_become_user_blocks() {
        let mut req = request();
        req.messages.push(
            ChatMessage::assistant("working").with_tool_calls(vec![reverie_core::llm::ToolCall {
                id: "toolu_1".into(),
                name: "web_search".into(),
                arguments: json!({"query": "rust"}),
            }]),
        );
        req.messages.push(ChatMessage::tool_result("toolu_1", "web_search", "results"));
        let body = build_body(&req);
        let messages = body["messages"].as_array().unwrap();
        let last = messages.last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["content"][0]["type"], "tool_result");
        assert_eq!(last["content"][0]["tool_use_id"], "toolu_1");
    }

    #[test]
    fn tools_use_input_schema() {
        let tool = ToolSpec::new("fs_read_file", "read", json!({"type":"object"}));
        let body = build_body(&request().with_tools(vec![tool]));
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["tools"][0]["name"], "fs_read_file");
    }

    #[test]
    fn assistant_tool_calls_become_tool_use() {
        let mut req = request();
        req.messages.push(
            ChatMessage::assistant("working").with_tool_calls(vec![reverie_core::llm::ToolCall {
                id: "toolu_9".into(),
                name: "shell_execute".into(),
                arguments: json!({"command": "ls"}),
            }]),
        );
        let body = build_body(&req);
        let blocks = body["messages"][1]["content"].as_array().unwrap();
        assert_eq!(blocks[1]["type"], "tool_use");
        assert_eq!(blocks[1]["id"], "toolu_9");
    }
}
