//! Ollama adapter: local daemon, `/api/chat` NDJSON streaming, `/api/tags` and
//! `/api/show` capability discovery.

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use reverie_core::domain::{Provider, Protocol};
use reverie_core::llm::{
    ChatMessage, ContentPart, FinishReason, LlmRequest, MessageRole, RemoteModel, StreamChunk, TokenUsage,
};
use serde_json::{json, Value};

use super::{flatten_parts, join_url, LlmAdapter};
use crate::error::{ensure_success, LlmError, LlmResult};
use crate::stream_util::{channel_pair, forward, ndjson_stream, ThinkTagSplitter};

/// Encode one message for `/api/chat`. Ollama takes images as a bare base64
/// array alongside a plain string `content`.
pub fn message_to_json(message: &ChatMessage) -> Value {
    let images: Vec<String> = message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ImageBase64 { data, .. } => Some(data.clone()),
            _ => None,
        })
        .collect();

    let text = flatten_parts(&message.parts);
    let role = match message.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::Tool => "tool",
    };

    let mut object = serde_json::Map::new();
    object.insert("role".to_string(), json!(role));
    object.insert("content".to_string(), json!(text));
    if !images.is_empty() {
        object.insert("images".to_string(), json!(images));
    }
    if message.role == MessageRole::Tool {
        if let Some(name) = &message.name {
            object.insert("tool_name".to_string(), json!(name));
        }
    }
    if !message.tool_calls.is_empty() {
        let calls: Vec<Value> = message
            .tool_calls
            .iter()
            .map(|call| json!({"function": {"name": call.name, "arguments": call.arguments}}))
            .collect();
        object.insert("tool_calls".to_string(), Value::Array(calls));
    }
    Value::Object(object)
}

/// Shape the request body. Pure for testability.
pub fn build_body(request: &LlmRequest) -> Value {
    let mut body = json!({
        "model": request.model,
        "messages": request.messages.iter().map(message_to_json).collect::<Vec<_>>(),
        "stream": true,
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
    }

    // Ollama exposes a boolean `think` flag for reasoning-capable local models.
    body["think"] = json!(request.thinking.is_reasoning());

    let mut options = serde_json::Map::new();
    if let Some(max_tokens) = request.max_tokens {
        options.insert("num_predict".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        options.insert("temperature".to_string(), json!(temperature));
    }
    if !options.is_empty() {
        body["options"] = Value::Object(options);
    }

    body
}

async fn pump(tx: tokio::sync::mpsc::Sender<LlmResult<StreamChunk>>, response: reqwest::Response) -> LlmResult<()> {
    let mut lines = ndjson_stream(response);
    let mut splitter = ThinkTagSplitter::new();
    let mut finished = false;

    while let Some(line) = lines.next().await {
        let chunk = line?;

        if let Some(error) = chunk.get("error").filter(|value| !value.is_null()) {
            return Err(LlmError::Protocol(format!("ollama error: {error}")));
        }

        let message = &chunk["message"];

        if let Some(thinking) = message.get("thinking").and_then(Value::as_str) {
            if !thinking.is_empty() && !forward(&tx, StreamChunk::ReasoningDelta(thinking.to_string())).await {
                return Ok(());
            }
        }

        if let Some(content) = message.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                let split = splitter.push(content);
                if !split.reasoning.is_empty()
                    && !forward(&tx, StreamChunk::ReasoningDelta(split.reasoning)).await
                {
                    return Ok(());
                }
                if !split.visible.is_empty() && !forward(&tx, StreamChunk::TextDelta(split.visible)).await {
                    return Ok(());
                }
            }
        }

        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for (index, call) in calls.iter().enumerate() {
                let function = &call["function"];
                let name = function.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                let arguments = match function.get("arguments") {
                    Some(Value::String(raw)) => raw.clone(),
                    Some(other) => other.to_string(),
                    None => "{}".to_string(),
                };
                let delta = StreamChunk::ToolCallDelta {
                    index,
                    id: None,
                    name: Some(name),
                    arguments_delta: arguments,
                };
                if !forward(&tx, delta).await {
                    return Ok(());
                }
            }
        }

        if chunk.get("done").and_then(Value::as_bool).unwrap_or(false) {
            let tail = splitter.finish();
            if !tail.reasoning.is_empty() {
                let _ = forward(&tx, StreamChunk::ReasoningDelta(tail.reasoning)).await;
            }
            if !tail.visible.is_empty() {
                let _ = forward(&tx, StreamChunk::TextDelta(tail.visible)).await;
            }

            let usage = TokenUsage {
                prompt_tokens: chunk.get("prompt_eval_count").and_then(Value::as_u64).unwrap_or(0) as u32,
                completion_tokens: chunk.get("eval_count").and_then(Value::as_u64).unwrap_or(0) as u32,
                reasoning_tokens: 0,
            };
            let _ = tx.send(Ok(StreamChunk::Usage(usage))).await;

            let reason = chunk.get("done_reason").and_then(Value::as_str).unwrap_or("stop");
            finished = true;
            let _ = tx
                .send(Ok(StreamChunk::Done { finish_reason: FinishReason::from_wire(reason) }))
                .await;
        }
    }

    if !finished {
        let _ = tx.send(Ok(StreamChunk::Done { finish_reason: FinishReason::Stop })).await;
    }
    Ok(())
}

/// Extract a context window from an Ollama `model_info` map.
pub fn context_length_from_info(info: &Value) -> Option<i64> {
    let object = info.as_object()?;
    object
        .iter()
        .find(|(key, _)| key.as_str() == "context_length" || key.ends_with(".context_length"))
        .and_then(|(_, value)| value.as_i64())
        .filter(|value| *value > 0)
}

/// Read the `capabilities` array into a normalized string list.
pub fn capabilities_of(show: &Value) -> Vec<String> {
    show.get("capabilities")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(|s| s.to_ascii_lowercase()).collect())
        .unwrap_or_default()
}

/// Adapter for a local Ollama daemon.
pub struct OllamaAdapter {
    client: reqwest::Client,
}

impl OllamaAdapter {
    pub fn new(client: reqwest::Client) -> Self {
        OllamaAdapter { client }
    }

    async fn enrich(&self, base: &str, mut model: RemoteModel) -> RemoteModel {
        let url = join_url(base, "/api/show");
        let request = self.client.post(&url).json(&json!({"name": model.model_name}));
        let Ok(response) = request.send().await else {
            return model;
        };
        let Ok(response) = ensure_success(response).await else {
            return model;
        };
        let Ok(show) = response.json::<Value>().await else {
            return model;
        };

        if let Some(context) = show.get("model_info").and_then(context_length_from_info) {
            model.context_window = Some(context);
        }
        let capabilities = capabilities_of(&show);
        if let Some(details) = show.get("details") {
            model.family = details.get("family").and_then(Value::as_str).map(str::to_string);
        }
        model.hints = json!({"capabilities": capabilities, "show": show});
        model
    }
}

#[async_trait]
impl LlmAdapter for OllamaAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Ollama
    }

    async fn stream_chat(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<BoxStream<'static, LlmResult<StreamChunk>>> {
        let url = join_url(&provider.normalized_base_url(), "/api/chat");
        let body = build_body(&request);
        let response = self.client.post(&url).json(&body).send().await?;
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
        let base = provider.normalized_base_url();
        let url = join_url(&base, "/api/tags");
        let response = self.client.get(&url).send().await?;
        let response = ensure_success(response).await?;
        let payload: Value = response.json().await?;

        let items = payload.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
        let discovered: Vec<RemoteModel> = items
            .iter()
            .filter_map(|item| {
                let name = item
                    .get("model")
                    .and_then(Value::as_str)
                    .or_else(|| item.get("name").and_then(Value::as_str))?
                    .to_string();
                let mut model = RemoteModel::new(name);
                model.owned_by = Some("ollama".to_string());
                model.hints = item.clone();
                Some(model)
            })
            .collect();

        let enriched: Vec<RemoteModel> = futures::stream::iter(
            discovered.into_iter().map(|model| self.enrich(&base, model)),
        )
        .buffer_unordered(8)
        .collect()
        .await;

        Ok(enriched)
    }
}

/// Convenience for callers that want the raw `/api/tags` payload shape.
pub async fn probe_daemon(client: &reqwest::Client, base: &str) -> LlmResult<Vec<String>> {
    let url = join_url(base, "/api/tags");
    let response = client.get(&url).send().await?;
    let response = ensure_success(response).await?;
    let payload: Value = response.json().await?;
    Ok(payload
        .get("models")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::llm::{ThinkingLevel, ToolSpec};

    fn request() -> LlmRequest {
        LlmRequest::new("llama3.1", vec![ChatMessage::system("sys"), ChatMessage::user("hi")])
    }

    #[test]
    fn body_streams_and_sets_think_flag() {
        let body = build_body(&request().with_thinking(ThinkingLevel::High));
        assert_eq!(body["stream"], true);
        assert_eq!(body["think"], true);
        let off = build_body(&request().with_thinking(ThinkingLevel::Off));
        assert_eq!(off["think"], false);
    }

    #[test]
    fn images_are_bare_base64_array() {
        let mut req = request();
        req.messages.push(ChatMessage::with_parts(
            MessageRole::User,
            vec![ContentPart::text("see"), ContentPart::image("image/png", "QUJD")],
        ));
        let body = build_body(&req);
        let message = &body["messages"][2];
        assert_eq!(message["images"][0], "QUJD");
        assert!(message["content"].as_str().unwrap().contains("see"));
    }

    #[test]
    fn options_carry_limits() {
        let body = build_body(&request().with_limits(Some(256), Some(0.2)));
        assert_eq!(body["options"]["num_predict"], 256);
        assert_eq!(body["options"]["temperature"], 0.2);
    }

    #[test]
    fn tools_use_openai_shape() {
        let tool = ToolSpec::new("t", "d", json!({"type":"object"}));
        let body = build_body(&request().with_tools(vec![tool]));
        assert_eq!(body["tools"][0]["function"]["name"], "t");
    }

    #[test]
    fn tool_messages_carry_tool_name() {
        let mut req = request();
        req.messages.push(ChatMessage::tool_result("x", "web_search", "ok"));
        let body = build_body(&req);
        let last = body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "tool");
        assert_eq!(last["tool_name"], "web_search");
    }

    #[test]
    fn context_length_scanning() {
        let info = json!({"llama.context_length": 131072, "llama.rope": 1});
        assert_eq!(context_length_from_info(&info), Some(131072));
        assert_eq!(context_length_from_info(&json!({})), None);
        assert_eq!(context_length_from_info(&json!({"context_length": 4096})), Some(4096));
    }

    #[test]
    fn capabilities_are_normalized() {
        let show = json!({"capabilities": ["Completion", "VISION", "tools"]});
        assert_eq!(capabilities_of(&show), vec!["completion", "vision", "tools"]);
        assert!(capabilities_of(&json!({})).is_empty());
    }
}
