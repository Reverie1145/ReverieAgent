//! Real-time events pushed from the Rust core to the desktop UI.
//!
//! Everything is serialized with an internal `kind` tag so the frontend can
//! switch on a single discriminant, and field names stay snake_case to keep the
//! TypeScript bridge trivial.

use serde::{Deserialize, Serialize};

/// Channel name used by the Tauri bridge for every event.
pub const EVENT_CHANNEL: &str = "reverie-event";

/// A single system event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SystemEvent {
    /// Incremental model output. `is_reasoning` separates thinking tokens from
    /// the visible answer.
    AgentStreamChunk { session_id: String, agent_id: String, delta_text: String, is_reasoning: bool },
    /// The current turn is being retried after a failed response: discard the
    /// partially streamed text so the retry does not appear duplicated.
    AgentStreamReset { session_id: String, agent_id: String },
    /// One-line human readable activity, e.g. `Searching web: "…"`.
    AgentActivity { session_id: String, agent_id: String, activity: String },
    /// A new sub-agent was spawned.
    AgentCreated {
        session_id: String,
        agent_id: String,
        role_name: String,
        archetype: String,
        model_id: String,
        thinking_level: String,
        status: String,
    },
    /// Lifecycle transition.
    AgentStatusChanged {
        session_id: String,
        agent_id: String,
        status: String,
        error_message: Option<String>,
    },
    /// The agent's model or thinking depth was switched at runtime.
    AgentRouteChanged {
        session_id: String,
        agent_id: String,
        model_id: String,
        thinking_level: String,
    },
    /// Token accounting delta.
    AgentTokenUsage {
        session_id: String,
        agent_id: String,
        prompt_tokens: i64,
        completion_tokens: i64,
        reasoning_tokens: i64,
    },
    /// A tool invocation started.
    AgentToolCall {
        session_id: String,
        agent_id: String,
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    /// A tool invocation finished.
    AgentToolResult {
        session_id: String,
        agent_id: String,
        call_id: String,
        tool_name: String,
        is_error: bool,
        preview: String,
    },
    /// Session-level lifecycle.
    SessionStatusChanged { session_id: String, status: String },
    /// The orchestrator reconfigured the swarm.
    SessionStrategyChanged {
        session_id: String,
        topology: String,
        context_strategy: String,
        isolation: String,
    },
    /// A finding was published to the blackboard.
    BlackboardPublished {
        session_id: String,
        entry_id: String,
        topic_tag: String,
        source_agent_id: String,
        summary: String,
    },
    /// A workspace file changed; carries a unified diff.
    WorkspaceDiffUpdated { session_id: String, agent_id: String, file_path: String, diff: String },
    /// Files changed on disk outside the tool pipeline (a human editor, an
    /// external build). Debounced, so one burst produces one event.
    WorkspaceFileChanged { session_id: String, paths: Vec<String> },
    /// Live web search progress.
    SearchProgress { session_id: String, agent_id: String, query: String, engine: String, results_found: usize },
    /// Model profiler progress.
    ProfilingProgress { provider_id: String, stage: String, message: String, completed: usize, total: usize },
    /// A model profile row changed.
    ModelProfileUpdated { model_id: String },
    /// The session produced its final deliverable.
    SessionCompleted { session_id: String, summary: String },
    /// Diagnostic log line surfaced in the UI activity feed.
    LogMessage { level: String, scope: String, message: String },
}

impl SystemEvent {
    /// The channel every event is emitted on.
    pub fn channel(&self) -> &'static str {
        EVENT_CHANNEL
    }

    /// Short discriminant, useful for filtering in logs.
    pub fn kind(&self) -> &'static str {
        match self {
            SystemEvent::AgentStreamChunk { .. } => "agent_stream_chunk",
            SystemEvent::AgentStreamReset { .. } => "agent_stream_reset",
            SystemEvent::AgentActivity { .. } => "agent_activity",
            SystemEvent::AgentCreated { .. } => "agent_created",
            SystemEvent::AgentStatusChanged { .. } => "agent_status_changed",
            SystemEvent::AgentRouteChanged { .. } => "agent_route_changed",
            SystemEvent::AgentTokenUsage { .. } => "agent_token_usage",
            SystemEvent::AgentToolCall { .. } => "agent_tool_call",
            SystemEvent::AgentToolResult { .. } => "agent_tool_result",
            SystemEvent::SessionStatusChanged { .. } => "session_status_changed",
            SystemEvent::SessionStrategyChanged { .. } => "session_strategy_changed",
            SystemEvent::BlackboardPublished { .. } => "blackboard_published",
            SystemEvent::WorkspaceDiffUpdated { .. } => "workspace_diff_updated",
            SystemEvent::WorkspaceFileChanged { .. } => "workspace_file_changed",
            SystemEvent::SearchProgress { .. } => "search_progress",
            SystemEvent::ProfilingProgress { .. } => "profiling_progress",
            SystemEvent::ModelProfileUpdated { .. } => "model_profile_updated",
            SystemEvent::SessionCompleted { .. } => "session_completed",
            SystemEvent::LogMessage { .. } => "log_message",
        }
    }

    /// The session this event belongs to, when it is session-scoped.
    pub fn session_id(&self) -> Option<&str> {
        match self {
            SystemEvent::AgentStreamChunk { session_id, .. }
            | SystemEvent::AgentStreamReset { session_id, .. }
            | SystemEvent::AgentActivity { session_id, .. }
            | SystemEvent::AgentCreated { session_id, .. }
            | SystemEvent::AgentStatusChanged { session_id, .. }
            | SystemEvent::AgentRouteChanged { session_id, .. }
            | SystemEvent::AgentTokenUsage { session_id, .. }
            | SystemEvent::AgentToolCall { session_id, .. }
            | SystemEvent::AgentToolResult { session_id, .. }
            | SystemEvent::SessionStatusChanged { session_id, .. }
            | SystemEvent::SessionStrategyChanged { session_id, .. }
            | SystemEvent::BlackboardPublished { session_id, .. }
            | SystemEvent::WorkspaceDiffUpdated { session_id, .. }
            | SystemEvent::WorkspaceFileChanged { session_id, .. }
            | SystemEvent::SearchProgress { session_id, .. }
            | SystemEvent::SessionCompleted { session_id, .. } => Some(session_id),
            SystemEvent::ProfilingProgress { .. }
            | SystemEvent::ModelProfileUpdated { .. }
            | SystemEvent::LogMessage { .. } => None,
        }
    }
}

/// Anything that can broadcast a [`SystemEvent`].
///
/// The runtime holds an `Arc<dyn EventSink>`; the Tauri layer supplies an
/// implementation backed by `AppHandle::emit`, while tests use [`NoopSink`].
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: SystemEvent);
}

/// Discards every event. Useful in unit tests and headless runs.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: SystemEvent) {}
}

/// Records events in memory, for assertions in tests.
#[derive(Debug, Default)]
pub struct RecordingSink {
    events: parking_lot::Mutex<Vec<SystemEvent>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Vec<SystemEvent> {
        self.events.lock().clone()
    }

    pub fn count(&self) -> usize {
        self.events.lock().len()
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: SystemEvent) {
        self.events.lock().push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_carry_a_kind_tag() {
        let ev = SystemEvent::AgentStreamChunk {
            session_id: "s".into(),
            agent_id: "a".into(),
            delta_text: "hi".into(),
            is_reasoning: true,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "agent_stream_chunk");
        assert_eq!(json["is_reasoning"], true);
        assert_eq!(ev.session_id(), Some("s"));
        assert_eq!(ev.channel(), "reverie-event");
    }

    #[test]
    fn non_session_events_have_no_session() {
        let ev = SystemEvent::ModelProfileUpdated { model_id: "m".into() };
        assert!(ev.session_id().is_none());
        assert_eq!(ev.kind(), "model_profile_updated");
    }

    #[test]
    fn recording_sink_collects() {
        let sink = RecordingSink::new();
        sink.emit(SystemEvent::LogMessage { level: "info".into(), scope: "t".into(), message: "m".into() });
        assert_eq!(sink.count(), 1);
        assert_eq!(sink.snapshot()[0].kind(), "log_message");
    }
}
