//! Live capability probing: measure real latency and detect which optional
//! request features a model accepts.

use std::time::Instant;

use reverie_core::domain::Provider;
use reverie_core::llm::{ChatMessage, ContentPart, LlmRequest, MessageRole, ThinkingLevel, ToolSpec};
use serde_json::json;

use crate::gateway::LlmGateway;

/// A 1×1 transparent PNG. Small enough to be free, real enough that a vision
/// model will accept it and a text-only model will reject the payload shape.
pub const TINY_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// Outcome of probing one model.
#[derive(Debug, Clone, Default)]
pub struct ProbeReport {
    /// Round-trip time of a minimal completion, in milliseconds.
    pub latency_ms: i64,
    /// `None` means "unknown, keep the heuristic".
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
    pub supports_thinking: Option<bool>,
    pub supported_thinking_levels: Option<Vec<ThinkingLevel>>,
    pub errors: Vec<String>,
    /// True when at least the baseline request succeeded.
    pub probed: bool,
}

impl ProbeReport {
    pub fn is_usable(&self) -> bool {
        self.probed
    }
}

/// Runs the probe battery against a single provider/model pair.
pub struct CapabilityProbe<'a> {
    gateway: &'a LlmGateway,
}

impl<'a> CapabilityProbe<'a> {
    pub fn new(gateway: &'a LlmGateway) -> Self {
        CapabilityProbe { gateway }
    }

    fn base_request(model: &str, prompt: &str, max_tokens: u32) -> LlmRequest {
        LlmRequest::new(model, vec![ChatMessage::user(prompt)])
            .with_thinking(ThinkingLevel::Off)
            .with_limits(Some(max_tokens), Some(0.0))
    }

    /// Probe one model. Never returns an error: failures are recorded in the
    /// report so the caller can fall back to heuristics.
    pub async fn run(&self, provider: &Provider, model: &str) -> ProbeReport {
        let mut report = ProbeReport::default();

        let started = Instant::now();
        match self.gateway.complete(provider, Self::base_request(model, "Reply with the single word: ready", 16)).await {
            Ok(_) => {
                report.latency_ms = started.elapsed().as_millis().max(1) as i64;
                report.probed = true;
            }
            Err(err) => {
                report.errors.push(format!("baseline failed: {err}"));
                return report;
            }
        }

        report.supports_tools = Some(self.probe_tools(provider, model).await);
        report.supports_vision = Some(self.probe_vision(provider, model).await);

        match self.probe_thinking(provider, model, ThinkingLevel::High).await {
            Some(true) => {
                let max_ok = self.probe_thinking(provider, model, ThinkingLevel::Max).await.unwrap_or(false);
                report.supports_thinking = Some(true);
                report.supported_thinking_levels = Some(if max_ok {
                    ThinkingLevel::all().to_vec()
                } else {
                    vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::Medium, ThinkingLevel::High]
                });
            }
            Some(false) => {
                report.supports_thinking = Some(false);
                report.supported_thinking_levels = Some(vec![ThinkingLevel::Off]);
            }
            None => {}
        }

        report
    }

    async fn probe_tools(&self, provider: &Provider, model: &str) -> bool {
        let tool = ToolSpec::new(
            "probe_echo",
            "Echo the provided text back to the caller.",
            json!({
                "type": "object",
                "properties": {"text": {"type": "string", "description": "text to echo"}},
                "required": ["text"]
            }),
        );
        let request = LlmRequest::new(
            model,
            vec![ChatMessage::user("Call the probe_echo tool with text set to ping.")],
        )
        .with_tools(vec![tool])
        .with_thinking(ThinkingLevel::Off)
        .with_limits(Some(64), Some(0.0));

        match self.gateway.complete(provider, request).await {
            Ok(_) => true,
            Err(err) if err.is_payload_rejection() => false,
            Err(_) => true,
        }
    }

    async fn probe_vision(&self, provider: &Provider, model: &str) -> bool {
        let message = ChatMessage::with_parts(
            MessageRole::User,
            vec![
                ContentPart::text("What colour is this pixel? Answer in one word."),
                ContentPart::image("image/png", TINY_PNG_BASE64),
            ],
        );
        let request = LlmRequest::new(model, vec![message])
            .with_thinking(ThinkingLevel::Off)
            .with_limits(Some(16), Some(0.0));

        match self.gateway.complete(provider, request).await {
            Ok(_) => true,
            Err(err) if err.is_payload_rejection() => false,
            Err(_) => false,
        }
    }

    /// `Some(true)` = accepted, `Some(false)` = rejected, `None` = inconclusive.
    async fn probe_thinking(&self, provider: &Provider, model: &str, level: ThinkingLevel) -> Option<bool> {
        let request = LlmRequest::new(model, vec![ChatMessage::user("Think briefly, then answer OK.")])
            .with_thinking(level)
            .with_limits(Some(512), Some(0.0));

        match self.gateway.complete(provider, request).await {
            Ok(_) => Some(true),
            Err(err) if err.is_payload_rejection() => Some(false),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_png_is_a_valid_data_payload() {
        let bytes = reverie_core::util::decode_base64(TINY_PNG_BASE64).expect("valid base64");
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
        assert!(bytes.len() > 60);
    }

    #[test]
    fn default_report_is_unusable() {
        let report = ProbeReport::default();
        assert!(!report.is_usable());
        assert!(report.supports_tools.is_none());
    }
}
