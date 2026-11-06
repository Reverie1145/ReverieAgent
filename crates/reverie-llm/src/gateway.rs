//! The gateway: one client, four protocols, a single streaming surface.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::BoxStream;
use futures::StreamExt;
use reverie_core::domain::{Provider, Protocol};
use reverie_core::llm::{FinishReason, LlmRequest, RemoteModel, StreamChunk, ToolCall, TokenUsage};

use crate::adapters::anthropic::AnthropicAdapter;
use crate::adapters::gemini::GeminiAdapter;
use crate::adapters::ollama::OllamaAdapter;
use crate::adapters::openai::OpenAiAdapter;
use crate::adapters::LlmAdapter;
use crate::error::{LlmError, LlmResult};
use crate::stream_util::ToolCallAccumulator;

/// Aggregated (non-streaming) completion.
#[derive(Debug, Clone, Default)]
pub struct Completion {
    pub text: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
    pub finish_reason: Option<FinishReason>,
}

impl Completion {
    /// Whether the model asked to call tools.
    pub fn wants_tools(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// Build the shared HTTP client with timeouts suitable for long generations.
pub fn build_client() -> LlmResult<reqwest::Client> {
    let client = reqwest::Client::builder()
        .user_agent("ReverieAgent/0.1 (+https://github.com/reverie-agent)")
        .connect_timeout(Duration::from_secs(20))
        // Streaming responses stay open for a long time, so only the connect and
        // read-idle phases are bounded.
        .read_timeout(Duration::from_secs(600))
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(32)
        .build()?;
    Ok(client)
}

/// Dispatches requests to the adapter matching the provider's protocol.
pub struct LlmGateway {
    client: reqwest::Client,
    openai: Arc<OpenAiAdapter>,
    anthropic: Arc<AnthropicAdapter>,
    gemini: Arc<GeminiAdapter>,
    ollama: Arc<OllamaAdapter>,
}

impl LlmGateway {
    /// Build a gateway with a freshly configured client.
    pub fn new() -> LlmResult<Self> {
        let client = build_client()?;
        Ok(LlmGateway::with_client(client))
    }

    /// Reuse an existing client (shared connection pool with the tool runtime).
    pub fn with_client(client: reqwest::Client) -> Self {
        LlmGateway {
            openai: Arc::new(OpenAiAdapter::new(client.clone())),
            anthropic: Arc::new(AnthropicAdapter::new(client.clone())),
            gemini: Arc::new(GeminiAdapter::new(client.clone())),
            ollama: Arc::new(OllamaAdapter::new(client.clone())),
            client,
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// The adapter for a protocol.
    pub fn adapter(&self, protocol: Protocol) -> &dyn LlmAdapter {
        match protocol {
            Protocol::OpenAiCompat => self.openai.as_ref(),
            Protocol::Anthropic => self.anthropic.as_ref(),
            Protocol::Gemini => self.gemini.as_ref(),
            Protocol::Ollama => self.ollama.as_ref(),
        }
    }

    /// Start a streaming completion.
    pub async fn stream_chat(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<BoxStream<'static, LlmResult<StreamChunk>>> {
        if !provider.is_enabled {
            return Err(LlmError::Other(format!("provider `{}` is disabled", provider.name)));
        }
        self.adapter(provider.protocol).stream_chat(provider, request).await
    }

    /// Pull the provider's live model list.
    pub async fn list_models(&self, provider: &Provider) -> LlmResult<Vec<RemoteModel>> {
        self.adapter(provider.protocol).list_models(provider).await
    }

    /// Drain a stream into a single [`Completion`].
    pub async fn complete(&self, provider: &Provider, request: LlmRequest) -> LlmResult<Completion> {
        let mut stream = self.stream_chat(provider, request).await?;
        let mut completion = Completion::default();
        let mut accumulator = ToolCallAccumulator::new();

        while let Some(item) = stream.next().await {
            match item? {
                StreamChunk::TextDelta(text) => completion.text.push_str(&text),
                StreamChunk::ReasoningDelta(text) => completion.reasoning.push_str(&text),
                StreamChunk::ToolCallDelta { index, id, name, arguments_delta } => {
                    accumulator.push(index, id.as_deref(), name.as_deref(), &arguments_delta);
                }
                StreamChunk::Usage(usage) => completion.usage.merge(usage),
                StreamChunk::Done { finish_reason } => {
                    completion.finish_reason = Some(finish_reason);
                }
            }
        }

        completion.tool_calls = accumulator.drain();
        Ok(completion)
    }

    /// `complete` with one retry for transient transport/provider failures.
    pub async fn complete_with_retry(
        &self,
        provider: &Provider,
        request: LlmRequest,
    ) -> LlmResult<Completion> {
        match self.complete(provider, request.clone()).await {
            Ok(value) => Ok(value),
            Err(err) if err.is_retryable() => {
                tokio::time::sleep(Duration::from_millis(700)).await;
                self.complete(provider, request).await
            }
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds() {
        assert!(build_client().is_ok());
    }

    #[test]
    fn adapter_dispatch_matches_protocol() {
        let gateway = LlmGateway::with_client(build_client().unwrap());
        assert_eq!(gateway.adapter(Protocol::OpenAiCompat).protocol(), Protocol::OpenAiCompat);
        assert_eq!(gateway.adapter(Protocol::Anthropic).protocol(), Protocol::Anthropic);
        assert_eq!(gateway.adapter(Protocol::Gemini).protocol(), Protocol::Gemini);
        assert_eq!(gateway.adapter(Protocol::Ollama).protocol(), Protocol::Ollama);
    }

    #[tokio::test]
    async fn disabled_provider_is_rejected() {
        let gateway = LlmGateway::with_client(build_client().unwrap());
        let mut provider = Provider::create("p", Protocol::OpenAiCompat, "https://api.example.com/v1", "sk-1");
        provider.is_enabled = false;
        let err = gateway
            .stream_chat(&provider, LlmRequest::new("m", vec![]))
            .await
            .err()
            .expect("disabled provider must fail");
        assert!(err.to_string().contains("disabled"));
    }

    #[test]
    fn completion_tool_detection() {
        let mut completion = Completion::default();
        assert!(!completion.wants_tools());
        completion.tool_calls.push(ToolCall {
            id: "c".into(),
            name: "n".into(),
            arguments: serde_json::json!({}),
        });
        assert!(completion.wants_tools());
    }
}
