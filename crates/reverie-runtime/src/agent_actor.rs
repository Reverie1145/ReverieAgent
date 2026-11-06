//! The agent actor loop: one Tokio task per sub-agent.
//!
//! Each iteration performs exactly one model call, streams text and reasoning
//! chunks to the UI, executes any requested tools, and feeds the results back.
//! Model and thinking depth are re-read every turn so the inspector can
//! re-route a running agent live.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use reverie_core::domain::{
    AgentRun, AgentStatus, ModelProfile, Provider, SearchEngineConfig, Session, TaskCategory,
};
use reverie_core::events::{EventSink, SystemEvent};
use reverie_core::llm::{
    ChatMessage, ContentPart, LlmRequest, MessageRole, StreamChunk, ThinkingLevel, TokenUsage, ToolCall,
};
use reverie_core::time;
use reverie_llm::stream_util::ToolCallAccumulator;
use reverie_llm::{LlmGateway, ModelRouter, RouteRequest};
use reverie_store::Store;
use reverie_tools::{GlobalToolRegistry, OrchestrationBridge, ToolExecutionContext, VfsLockManager};

use crate::control::AgentControl;

/// How often a paused agent re-checks its state.
const PAUSE_POLL: Duration = Duration::from_millis(250);
/// Failed model responses are retried this many times before the agent fails.
const MAX_RESPONSE_RETRIES: usize = 5;
/// First backoff delay; doubled on every retry, capped at [`RETRY_MAX_DELAY`].
const RETRY_BASE_DELAY: Duration = Duration::from_millis(1_000);
const RETRY_MAX_DELAY: Duration = Duration::from_millis(20_000);

/// Shared services every actor needs.
pub struct ActorDeps {
    pub store: Arc<Store>,
    pub gateway: Arc<LlmGateway>,
    pub registry: Arc<GlobalToolRegistry>,
    pub sink: Arc<dyn EventSink>,
    pub vfs: Arc<VfsLockManager>,
    pub http: reqwest::Client,
    pub orchestration: Arc<dyn OrchestrationBridge>,
}

/// Everything needed to start one agent.
pub struct ActorInput {
    pub run: AgentRun,
    pub session: Session,
    pub system_prompt: String,
    pub briefing: String,
    pub allowed_tools: Option<Vec<String>>,
    pub category: TaskCategory,
    pub complexity: f32,
    pub control: Arc<AgentControl>,
    pub web_search_enabled: bool,
    pub search_config: SearchEngineConfig,
}

/// Terminal state of one agent.
#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub status: AgentStatus,
    pub output: Option<String>,
    pub error: Option<String>,
    pub usage: TokenUsage,
}

impl AgentOutcome {
    fn failed(message: impl Into<String>, usage: TokenUsage) -> Self {
        AgentOutcome { status: AgentStatus::Failed, output: None, error: Some(message.into()), usage }
    }

    fn cancelled(usage: TokenUsage) -> Self {
        AgentOutcome { status: AgentStatus::Cancelled, output: None, error: Some("cancelled".into()), usage }
    }
}

/// Replace inline image payloads with a placeholder before persisting, so a
/// long session does not turn the SQLite file into a photo album.
pub fn compact_for_storage(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| {
            let parts = message
                .parts
                .iter()
                .map(|part| match part {
                    ContentPart::ImageBase64 { mime_type, .. } => ContentPart::ImageBase64 {
                        mime_type: mime_type.clone(),
                        data: "<stripped>".to_string(),
                    },
                    other => other.clone(),
                })
                .collect();
            ChatMessage { parts, ..message.clone() }
        })
        .collect()
}

async fn pick_route(
    deps: &ActorDeps,
    input: &ActorInput,
) -> Result<(Provider, ModelProfile, ThinkingLevel), String> {
    let profiles = deps.store.list_model_profiles().await.map_err(|err| err.to_string())?;
    if profiles.is_empty() {
        return Err("no model profiles configured — add a provider and pull its model list first".to_string());
    }

    let requested = input.control.model_id();
    if let Some(profile) = profiles.iter().find(|candidate| candidate.id == requested) {
        if let Ok(Some(provider)) = deps.store.get_provider(&profile.provider_id).await {
            if provider.is_enabled {
                let thinking = profile.resolve_thinking(input.control.thinking());
                return Ok((provider, profile.clone(), thinking));
            }
        }
    }

    let route = RouteRequest::new(input.category, input.complexity)
        .with_tools(true)
        .with_override(Some(input.control.thinking()));
    let decision = ModelRouter::select(&profiles, &route).map_err(|err| err.to_string())?;
    let provider = deps
        .store
        .get_provider(&decision.profile.provider_id)
        .await
        .map_err(|err| err.to_string())?
        .ok_or_else(|| format!("provider for model {} is missing", decision.profile.model_name))?;

    input.control.set_model_id(decision.profile.id.clone());
    input.control.set_thinking(decision.thinking);
    Ok((provider, decision.profile, decision.thinking))
}

fn max_output_tokens(profile: &ModelProfile) -> u32 {
    let quarter = profile.effective_context_window() / 4;
    quarter.clamp(1024, 16_384) as u32
}

/// Exponential backoff for a failed response attempt: 1s, 2s, 4s, 8s, 16s.
fn retry_delay(attempt: usize) -> Duration {
    let factor = 1u32 << attempt.saturating_sub(1).min(5);
    (RETRY_BASE_DELAY * factor).min(RETRY_MAX_DELAY)
}

/// Sleep for `delay`, returning `false` if the run is cancelled meanwhile.
async fn wait_or_cancel(control: &AgentControl, delay: Duration) -> bool {
    let token = control.token();
    tokio::select! {
        _ = tokio::time::sleep(delay) => true,
        _ = token.cancelled() => false,
    }
}

/// Run one agent to completion.
pub async fn run_agent(deps: Arc<ActorDeps>, input: ActorInput) -> AgentOutcome {
    let session_id = input.session.id.clone();
    let agent_id = input.run.id.clone();
    let mut usage = TokenUsage::default();

    let (provider, profile, initial_thinking) = match pick_route(&deps, &input).await {
        Ok(route) => route,
        Err(err) => return AgentOutcome::failed(err, usage),
    };

    deps.sink.emit(SystemEvent::AgentRouteChanged {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        model_id: profile.id.clone(),
        thinking_level: initial_thinking.as_str().to_string(),
    });

    let tool_specs = deps.registry.specs(input.allowed_tools.as_deref());
    let tool_names: Vec<String> = tool_specs.iter().map(|spec| spec.name.clone()).collect();

    let mut messages = ChatMessage::decode_history(&input.run.message_history_json);
    if messages.is_empty() {
        messages.push(ChatMessage::system(&input.system_prompt));
        if !input.briefing.trim().is_empty() {
            messages.push(ChatMessage::user(&input.briefing));
        }
    }

    let ctx = ToolExecutionContext {
        session_id: session_id.clone(),
        agent_id: agent_id.clone(),
        role_name: input.run.role_name.clone(),
        workspace_dir: std::path::PathBuf::from(&input.run.workspace_dir),
        store: deps.store.clone(),
        vfs: deps.vfs.clone(),
        sink: deps.sink.clone(),
        http: deps.http.clone(),
        search_config: input.search_config.clone(),
        web_search_enabled: input.web_search_enabled,
        registry: deps.registry.clone(),
        orchestration: Some(deps.orchestration.clone()),
        max_output_chars: reverie_tools::DEFAULT_MAX_OUTPUT_CHARS,
    };

    let mut turn = 0usize;

    // Tool-calling rounds are unbounded: an agent keeps working for as long as
    // it needs. The loop only ends when the model stops calling tools, the run
    // is cancelled, or a response fails after every retry.
    let final_text: String = loop {
        if input.control.is_cancelled() {
            return AgentOutcome::cancelled(usage);
        }
        while input.control.is_paused() {
            if input.control.is_cancelled() {
                return AgentOutcome::cancelled(usage);
            }
            tokio::time::sleep(PAUSE_POLL).await;
        }

        turn += 1;
        let thinking = input.control.thinking();
        let model_id = input.control.model_id();
        let (provider, profile) = if model_id != profile.id {
            match pick_route(&deps, &input).await {
                Ok((provider, profile, _)) => (provider, profile),
                Err(err) => return AgentOutcome::failed(err, usage),
            }
        } else {
            (provider.clone(), profile.clone())
        };

        let request = LlmRequest::new(&profile.model_name, messages.clone())
            .with_tools(tool_specs.clone())
            .with_thinking(thinking)
            .with_limits(Some(max_output_tokens(&profile)), Some(0.3));

        // One turn, retried up to MAX_RESPONSE_RETRIES times on failure.
        let mut attempt = 0usize;
        let mut text = String::new();
        let mut reasoning = String::new();

        let calls: Vec<ToolCall> = 'retry: loop {
            attempt += 1;
            text.clear();
            reasoning.clear();
            let mut accumulator = ToolCallAccumulator::new();

            deps.sink.emit(SystemEvent::AgentActivity {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                activity: if attempt == 1 {
                    format!(
                        "Thinking with {} ({}), round {turn}",
                        profile.model_name,
                        thinking.as_str()
                    )
                } else {
                    format!("Retrying response {attempt}/{MAX_RESPONSE_RETRIES}…")
                },
            });

            let mut failure: Option<String> = None;

            match deps.gateway.stream_chat(&provider, request.clone()).await {
                Ok(mut stream) => {
                    while let Some(item) = stream.next().await {
                        if input.control.is_cancelled() {
                            return AgentOutcome::cancelled(usage);
                        }
                        match item {
                            Ok(StreamChunk::TextDelta(delta)) => {
                                text.push_str(&delta);
                                deps.sink.emit(SystemEvent::AgentStreamChunk {
                                    session_id: session_id.clone(),
                                    agent_id: agent_id.clone(),
                                    delta_text: delta,
                                    is_reasoning: false,
                                });
                            }
                            Ok(StreamChunk::ReasoningDelta(delta)) => {
                                reasoning.push_str(&delta);
                                deps.sink.emit(SystemEvent::AgentStreamChunk {
                                    session_id: session_id.clone(),
                                    agent_id: agent_id.clone(),
                                    delta_text: delta,
                                    is_reasoning: true,
                                });
                            }
                            Ok(StreamChunk::ToolCallDelta { index, id, name, arguments_delta }) => {
                                accumulator.push(index, id.as_deref(), name.as_deref(), &arguments_delta);
                            }
                            Ok(StreamChunk::Usage(delta)) => {
                                // Tokens already spent are real even if the turn
                                // is retried, so the counter is never rolled back.
                                usage.merge(delta);
                                deps.sink.emit(SystemEvent::AgentTokenUsage {
                                    session_id: session_id.clone(),
                                    agent_id: agent_id.clone(),
                                    prompt_tokens: usage.prompt_tokens as i64,
                                    completion_tokens: usage.completion_tokens as i64,
                                    reasoning_tokens: usage.reasoning_tokens as i64,
                                });
                            }
                            Ok(StreamChunk::Done { .. }) => {}
                            Err(err) => {
                                failure = Some(err.to_string());
                                break;
                            }
                        }
                    }
                }
                Err(err) => failure = Some(err.to_string()),
            }

            let Some(error) = failure else {
                break 'retry accumulator.drain();
            };

            if attempt > MAX_RESPONSE_RETRIES {
                let _ = deps.store.record_model_outcome(&profile.id, false, 0).await;
                return AgentOutcome::failed(
                    format!("response failed after {MAX_RESPONSE_RETRIES} retries: {error}"),
                    usage,
                );
            }

            let delay = retry_delay(attempt);
            deps.sink.emit(SystemEvent::AgentActivity {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                activity: format!(
                    "Response failed ({error}); retrying in {}s…",
                    delay.as_secs().max(1)
                ),
            });
            if !wait_or_cancel(&input.control, delay).await {
                return AgentOutcome::cancelled(usage);
            }
            // Drop the partially streamed text so the retry is not duplicated.
            deps.sink.emit(SystemEvent::AgentStreamReset {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
            });
        };

        let mut parts: Vec<ContentPart> = Vec::new();
        if !reasoning.trim().is_empty() {
            parts.push(ContentPart::reasoning(reasoning.clone()));
        }
        if !text.is_empty() {
            parts.push(ContentPart::text(text.clone()));
        }
        messages.push(ChatMessage::with_parts(MessageRole::Assistant, parts).with_tool_calls(calls.clone()));

        if calls.is_empty() {
            break text;
        }
        for call in calls {
            if input.control.is_cancelled() {
                return AgentOutcome::cancelled(usage);
            }
            let known = tool_names.iter().any(|name| name == &call.name);
            deps.sink.emit(SystemEvent::AgentToolCall {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                call_id: call.id.clone(),
                tool_name: call.name.clone(),
                arguments: call.arguments.clone(),
            });

            let outcome = if known {
                deps.registry.execute(&call.name, &ctx, call.arguments.clone()).await
            } else {
                reverie_core::domain::ToolOutcome::error(format!(
                    "tool `{}` is not available to this agent. Available: {}",
                    call.name,
                    tool_names.join(", ")
                ))
            };

            deps.sink.emit(SystemEvent::AgentToolResult {
                session_id: session_id.clone(),
                agent_id: agent_id.clone(),
                call_id: call.id.clone(),
                tool_name: call.name.clone(),
                is_error: outcome.is_error,
                preview: reverie_core::util::truncate_chars(outcome.content.trim(), 240),
            });

            messages.push(ChatMessage::tool_result(&call.id, &call.name, outcome.to_message_content()));

            if let (Some(data), Some(mime)) = (
                outcome.metadata.get("image_base64").and_then(|value| value.as_str()),
                outcome.metadata.get("mime_type").and_then(|value| value.as_str()),
            ) {
                messages.push(ChatMessage::with_parts(
                    MessageRole::User,
                    vec![
                        ContentPart::text(format!("Image returned by `{}`:", call.name)),
                        ContentPart::image(mime, data),
                    ],
                ));
            }
        }

        let compact = compact_for_storage(&messages);
        if let Ok(json) = ChatMessage::encode_history(&compact) {
            let _ = deps
                .store
                .save_agent_progress(
                    &agent_id,
                    &json,
                    usage.prompt_tokens as i64,
                    usage.completion_tokens as i64,
                    usage.reasoning_tokens as i64,
                )
                .await;
        }
    };

    let compact = compact_for_storage(&messages);
    if let Ok(json) = ChatMessage::encode_history(&compact) {
        let _ = deps
            .store
            .save_agent_progress(
                &agent_id,
                &json,
                usage.prompt_tokens as i64,
                usage.completion_tokens as i64,
                usage.reasoning_tokens as i64,
            )
            .await;
    }

    if final_text.trim().is_empty() {
        return AgentOutcome::failed(
            format!("agent stopped after {turn} turn(s) without producing a final answer"),
            usage,
        );
    }

    let _ = deps.store.record_model_outcome(&profile.id, true, 0).await;
    AgentOutcome { status: AgentStatus::Completed, output: Some(final_text), error: None, usage }
}

/// Mark an agent as running and stamp its start time.
pub async fn mark_running(store: &Store, agent_id: &str) -> bool {
    store
        .update_agent_status(agent_id, AgentStatus::Running, None)
        .await
        .map(|_| true)
        .unwrap_or(false)
}

/// Elapsed wall-clock milliseconds since the run started.
pub fn elapsed(run: &AgentRun) -> i64 {
    time::elapsed_ms(run.created_at, time::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_payloads_are_stripped_for_storage() {
        let messages = vec![ChatMessage::with_parts(
            MessageRole::User,
            vec![ContentPart::text("see"), ContentPart::image("image/png", "HUGEBASE64")],
        )];
        let compact = compact_for_storage(&messages);
        let encoded = ChatMessage::encode_history(&compact).unwrap();
        assert!(!encoded.contains("HUGEBASE64"));
        assert!(encoded.contains("<stripped>"));
        assert!(encoded.contains("image/png"));
    }

    #[test]
    fn max_output_scales_with_window() {
        let mut profile = ModelProfile::provisional("p", "m");
        profile.context_window = 1000;
        assert_eq!(max_output_tokens(&profile), 1024);
        profile.context_window = 200_000;
        assert_eq!(max_output_tokens(&profile), 16_384);
    }

    #[test]
    fn outcome_constructors_set_status() {
        assert_eq!(AgentOutcome::failed("x", TokenUsage::default()).status, AgentStatus::Failed);
        assert_eq!(AgentOutcome::cancelled(TokenUsage::default()).status, AgentStatus::Cancelled);
    }

    #[test]
    fn response_retries_are_five() {
        assert_eq!(MAX_RESPONSE_RETRIES, 5);
    }

    #[test]
    fn retry_backoff_doubles_then_caps() {
        assert_eq!(retry_delay(1), Duration::from_millis(1_000));
        assert_eq!(retry_delay(2), Duration::from_millis(2_000));
        assert_eq!(retry_delay(3), Duration::from_millis(4_000));
        assert_eq!(retry_delay(5), Duration::from_millis(16_000));
        // Beyond the cap the delay stops growing.
        assert_eq!(retry_delay(9), RETRY_MAX_DELAY);
    }
}
