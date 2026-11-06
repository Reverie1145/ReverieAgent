//! Google Gemini adapter (`generateContent` / `streamGenerateContent`).

use async_trait::async_trait;
use futures::stream::BoxStream;
use futures::StreamExt;
use reverie_core::domain::{Provider, Protocol};
use reverie_core::llm::{
    ChatMessage, ContentPart, FinishReason, LlmRequest, MessageRole, RemoteModel, StreamChunk, TokenUsage,
};
use serde_json::{json, Map, Value};

use super::{document_block, join_url, LlmAdapter};
use crate::error::{ensure_success, LlmError, LlmResult};
use crate::stream_util::{channel_pair, forward, sse_stream};

/// Keys Gemini's `Schema` type does not accept; sending them yields HTTP 400.
const UNSUPPORTED_SCHEMA_KEYS: [&str; 18] = [
    "additionalProperties",
    "$schema",
    "$id",
    "$ref",
    "definitions",
    "$defs",
    "examples",
    "const",
    "oneOf",
    "anyOf",
    "allOf",
    "not",
    "patternProperties",
    "dependencies",
    "if",
    "else",
    "default",
    "title",
];

/// Recursively strip JSON-Schema constructs Gemini rejects.
pub fn sanitize_schema(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, value) in map {
                if UNSUPPORTED_SCHEMA_KEYS.contains(&key.as_str()) {
                    continue;
                }
                match key.as_str() {
                    "properties" => {
                        if let Value::Object(props) = value {
                            let mut cleaned = Map::new();
                            for (prop_name, prop_schema) in props {
                                cleaned.insert(prop_name.clone(), sanitize_schema(prop_schema));
                            }
                            out.insert(key.clone(), Value::Object(cleaned));
                        } else {
                            out.insert(key.clone(), Value::Object(Map::new()));
                        }
                    }
                    "items" => {
                        out.insert(key.clone(), sanitize_schema(value));
                    }
                    _ => {
                        out.insert(key.clone(), value.clone());
                    }
                }
            }
            if out.get("type").and_then(Value::as_str) == Some("object") && !out.contains_key("properties") {
                out.insert("properties".to_string(), Value::Object(Map::new()));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_schema).collect()),
        other => other.clone(),
    }
}

fn parts_for(message: &ChatMessage) -> Vec<Value> {
    let mut parts: Vec<Value> = Vec::new();
    for part in &message.parts {
        match part {
            ContentPart::Text { text } => {
                if !text.is_empty() {
                    parts.push(json!({"text": text}));
                }
            }
            ContentPart::ImageBase64 { mime_type, data } => {
                parts.push(json!({"inlineData": {"mimeType": mime_type, "data": data}}));
            }
            ContentPart::FileDocument { file_name, mime_type, content } => {
                parts.push(json!({"text": document_block(file_name, mime_type, content)}));
            }
            ContentPart::Reasoning { .. } => {}
        }
    }
    for call in &message.tool_calls {
        parts.push(json!({"functionCall": {"name": call.name, "args": call.arguments}}));
    }
    parts
}

/// Convert the transcript into Gemini `contents`, hoisting the system prompt.
pub fn build_contents(messages: &[&ChatMessage]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        let (role, parts) = match message.role {
            MessageRole::Assistant => ("model", parts_for(message)),
            MessageRole::Tool => {
                let name = message.name.clone().unwrap_or_else(|| "tool".to_string());
                (
                    "user",
                    vec![json!({
                        "functionResponse": {"name": name, "response": {"result": message.text_content()}}
                    })],
                )
            }
            _ => ("user", parts_for(message)),
        };
        if parts.is_empty() {
            continue;
        }
        if let Some(last) = out.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some(role) {
                if let Some(existing) = last.get_mut("parts").and_then(Value::as_array_mut) {
                    existing.extend(parts);
                    continue;
                }
            }
        }
        out.push(json!({"role": role, "parts": parts}));
    }
    out
}

/// Shape the request body. Pure for testability.
pub fn build_body(request: &LlmRequest) -> Value {
    let (system, rest) = request.split_system();
    let mut body = json!({"contents": build_contents(&rest)});

    if let Some(system) = system {
        body["systemInstruction"] = json!({"parts": [{"text": system}]});
    }

    if !request.tools.is_empty() {
        let declarations: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": sanitize_schema(&tool.parameters),
                })
            })
            .collect();
        body["tools"] = json!([{"functionDeclarations": declarations}]);
    }

    let mut config = Map::new();
    if let Some(max_tokens) = request.max_tokens {
        config.insert("maxOutputTokens".to_string(), json!(max_tokens));
    }
    if let Some(temperature) = request.temperature {
        config.insert("temperature".to_string(), json!(temperature));
    }
    if request.thinking.is_reasoning() {
        config.insert(
            "thinkingConfig".to_string(),
            json!({
                "thinkingBudget": request.thinking.gemini_thinking_budget(),
                "includeThoughts": true
            }),
        );
    } else if request.thinking == reverie_core::llm::ThinkingLevel::Off {
        config.insert(
            "thinkingConfig".to_string(),
            json!({"thinkingBudget": 0, "includeThoughts": false}),
        );
    }
    if !config.is_empty() {
        body["generationConfig"] = Value::Object(config);
    }

    body
}

/// Gemini model identifiers are prefixed with `models/`; strip it.
pub fn strip_model_prefix(name: &str) -> String {
    name.trim().trim_start_matches("models/").to_string()
}

async fn pump(tx: tokio::sync::mpsc::Sender<LlmResult<StreamChunk>>, response: reqwest::Response) -> LlmResult<()> {
    let mut events = sse_stream(response);
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

        if let Some(error) = chunk.get("error").filter(|value| !value.is_null()) {
            return Err(LlmError::Protocol(format!("gemini stream error: {error}")));
        }

        if let Some(usage) = chunk.get("usageMetadata") {
            let _ = tx
                .send(Ok(StreamChunk::Usage(TokenUsage {
                    prompt_tokens: usage.get("promptTokenCount").and_then(Value::as_u64).unwrap_or(0) as u32,
                    completion_tokens: usage
                        .get("candidatesTokenCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32,
                    reasoning_tokens: usage
                        .get("thoughtsTokenCount")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32,
                })))
                .await;
        }

        let Some(candidate) = chunk.get("candidates").and_then(|value| value.get(0)) else {
            continue;
        };

        if let Some(parts) = candidate.get("content").and_then(|content| content.get("parts")).and_then(Value::as_array)
        {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    if text.is_empty() {
                        continue;
                    }
                    let is_thought = part.get("thought").and_then(Value::as_bool).unwrap_or(false);
                    let chunk = if is_thought {
                        StreamChunk::ReasoningDelta(text.to_string())
                    } else {
                        StreamChunk::TextDelta(text.to_string())
                    };
                    if !forward(&tx, chunk).await {
                        return Ok(());
                    }
                }
                if let Some(call) = part.get("functionCall") {
                    let name = call.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                    let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
                    let delta = StreamChunk::ToolCallDelta {
                        index: 0,
                        id: None,
                        name: Some(name),
                        arguments_delta: args.to_string(),
                    };
                    if !forward(&tx, delta).await {
                        return Ok(());
                    }
                }
            }
        }

        if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
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

/// Adapter for the Google Generative Language API.
pub struct GeminiAdapter {
    client: reqwest::Client,
}

impl GeminiAdapter {
    pub fn new(client: reqwest::Client) -> Self {
        GeminiAdapter { client }
    }
}

#[async_trait]
impl LlmAdapter for GeminiAdapter {
    fn protocol(&self) -> Protocol {
        Protocol::Gemini
    }

    async fn stream_chat(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<BoxStream<'static, LlmResult<StreamChunk>>> {
        let model = strip_model_prefix(&request.model);
        let path = format!("/models/{model}:streamGenerateContent?alt=sse&key={}", provider.api_key.trim());
        let url = join_url(&provider.normalized_base_url(), &path);
        let body = build_body(&request);

        let response = self
            .client
            .post(&url)
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
        let path = format!("/models?pageSize=1000&key={}", provider.api_key.trim());
        let url = join_url(&provider.normalized_base_url(), &path);
        let response = self.client.get(&url).send().await?;
        let response = ensure_success(response).await?;
        let payload: Value = response.json().await?;

        let items = payload.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|item| {
                let raw_name = item.get("name").and_then(Value::as_str)?;
                let name = strip_model_prefix(raw_name);
                if name.is_empty() {
                    return None;
                }
                // Only chat-capable models are useful to the swarm.
                let supports_generate = item
                    .get("supportedGenerationMethods")
                    .and_then(Value::as_array)
                    .map(|methods| methods.iter().any(|m| m.as_str() == Some("generateContent")))
                    .unwrap_or(true);
                if !supports_generate {
                    return None;
                }
                let mut model = RemoteModel::new(name);
                model.display_name = item.get("displayName").and_then(Value::as_str).map(str::to_string);
                model.context_window = item.get("inputTokenLimit").and_then(Value::as_i64);
                model.family = item.get("displayName").and_then(Value::as_str).map(str::to_string);
                model.owned_by = Some("google".to_string());
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
        LlmRequest::new("gemini-2.5-pro", vec![ChatMessage::system("sys"), ChatMessage::user("hi")])
    }

    #[test]
    fn schema_sanitizer_strips_unsupported_keys() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "$schema": "http://json-schema.org/draft-07/schema#",
            "properties": {
                "query": {"type": "string", "default": "x"},
                "nested": {"type": "object", "properties": {"a": {"type": "integer", "title": "A"}}}
            }
        });
        let cleaned = sanitize_schema(&schema);
        assert!(cleaned.get("additionalProperties").is_none());
        assert!(cleaned.get("$schema").is_none());
        assert!(cleaned["properties"]["query"].get("default").is_none());
        assert!(cleaned["properties"]["nested"]["properties"]["a"].get("title").is_none());
    }

    #[test]
    fn empty_object_gets_properties() {
        let cleaned = sanitize_schema(&json!({"type": "object"}));
        assert_eq!(cleaned["properties"], json!({}));
    }

    #[test]
    fn system_instruction_is_hoisted() {
        let body = build_body(&request());
        assert_eq!(body["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn thinking_config_is_set() {
        let body = build_body(&request().with_thinking(ThinkingLevel::High));
        assert_eq!(body["generationConfig"]["thinkingConfig"]["thinkingBudget"], 24576);
        assert_eq!(body["generationConfig"]["thinkingConfig"]["includeThoughts"], true);
        let off = build_body(&request().with_thinking(ThinkingLevel::Off));
        assert_eq!(off["generationConfig"]["thinkingConfig"]["thinkingBudget"], 0);
    }

    #[test]
    fn images_become_inline_data() {
        let mut req = request();
        req.messages.push(ChatMessage::with_parts(
            MessageRole::User,
            vec![ContentPart::image("image/webp", "AAAA")],
        ));
        let body = build_body(&req);
        // Consecutive user turns are merged into one `contents` entry.
        let parts = body["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/webp");
        assert_eq!(parts[1]["inlineData"]["data"], "AAAA");
    }

    #[test]
    fn tools_are_sanitized() {
        let tool = ToolSpec::new("t", "d", json!({"type":"object","additionalProperties": true}));
        let body = build_body(&request().with_tools(vec![tool]));
        let params = &body["tools"][0]["functionDeclarations"][0]["parameters"];
        assert!(params.get("additionalProperties").is_none());
    }

    #[test]
    fn tool_results_become_function_response() {
        let mut req = request();
        req.messages.push(
            ChatMessage::assistant("calling").with_tool_calls(vec![reverie_core::llm::ToolCall {
                id: "c1".into(),
                name: "web_search".into(),
                arguments: json!({"query": "rust"}),
            }]),
        );
        req.messages.push(ChatMessage::tool_result("c1", "web_search", "ok"));
        let body = build_body(&req);
        let last = body["contents"].as_array().unwrap().last().unwrap();
        assert_eq!(last["role"], "user");
        assert_eq!(last["parts"][0]["functionResponse"]["name"], "web_search");
        assert_eq!(last["parts"][0]["functionResponse"]["response"]["result"], "ok");
    }

    #[test]
    fn model_prefix_is_stripped() {
        assert_eq!(strip_model_prefix("models/gemini-2.5-flash"), "gemini-2.5-flash");
        assert_eq!(strip_model_prefix("gemini-pro"), "gemini-pro");
    }
}
