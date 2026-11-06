//! Read models handed to the desktop UI.

use reverie_core::domain::{AgentRun, BlackboardEntry, Session, SessionAttachment};
use reverie_core::llm::ChatMessage;
use serde::{Deserialize, Serialize};

/// Aggregate token counters for a session.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct TokenTotals {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub reasoning_tokens: i64,
}

/// One agent card in the swarm grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub session_id: String,
    pub parent_agent_id: Option<String>,
    pub role_name: String,
    pub archetype: String,
    pub archetype_label: String,
    pub model_id: String,
    pub model_name: String,
    pub thinking_level: String,
    pub status: String,
    pub task_description: String,
    pub workspace_dir: String,
    pub git_branch: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub reasoning_tokens: i64,
    pub elapsed_ms: i64,
    pub activity: String,
    pub error_message: Option<String>,
    pub depends_on: Vec<String>,
    pub attached_file_ids: Vec<String>,
    pub output_preview: Option<String>,
}

impl AgentSummary {
    /// Build a card from a persisted run, optionally enriched with live state.
    pub fn from_run(run: &AgentRun, model_name: Option<String>, activity: Option<String>) -> Self {
        let output_preview = run
            .output_artifact
            .as_ref()
            .map(|artifact| reverie_core::util::truncate_chars(artifact.trim(), 600));
        AgentSummary {
            id: run.id.clone(),
            session_id: run.session_id.clone(),
            parent_agent_id: run.parent_agent_id.clone(),
            role_name: run.role_name.clone(),
            archetype: run.archetype.as_str().to_string(),
            archetype_label: run.archetype.label().to_string(),
            model_id: run.model_id.clone(),
            model_name: model_name.unwrap_or_else(|| short_model(&run.model_id)),
            thinking_level: run.thinking_level.as_str().to_string(),
            status: run.status.as_str().to_string(),
            task_description: run.task_description.clone(),
            workspace_dir: run.workspace_dir.clone(),
            git_branch: run.git_branch.clone(),
            prompt_tokens: run.prompt_tokens,
            completion_tokens: run.completion_tokens,
            reasoning_tokens: run.reasoning_tokens,
            elapsed_ms: run.elapsed_ms(),
            activity: activity.unwrap_or_default(),
            error_message: run.error_message.clone(),
            depends_on: run.depend_on_agent_ids.clone(),
            attached_file_ids: run.attached_file_ids.clone(),
            output_preview,
        }
    }
}

/// `provider:model` → `model` for compact badges.
pub fn short_model(model_id: &str) -> String {
    match model_id.split_once(':') {
        Some((_, model)) => model.to_string(),
        None => model_id.to_string(),
    }
}

/// Everything the task stage needs to render a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session: Session,
    pub attachments: Vec<SessionAttachment>,
    pub agents: Vec<AgentSummary>,
    pub blackboard: Vec<BlackboardEntry>,
    pub final_summary: Option<String>,
    pub totals: TokenTotals,
    pub active: bool,
}

/// The inspector drawer payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDetail {
    pub agent: AgentSummary,
    pub messages: Vec<ChatMessage>,
    pub attachments: Vec<SessionAttachment>,
    pub published: Vec<BlackboardEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::{AgentArchetype, AgentSpec};

    #[test]
    fn short_model_strips_provider() {
        assert_eq!(short_model("prov-1:gpt-4o"), "gpt-4o");
        assert_eq!(short_model("gpt-4o"), "gpt-4o");
        assert_eq!(short_model(""), "");
    }

    #[test]
    fn summary_previews_long_output() {
        let spec = AgentSpec::new("Writer", AgentArchetype::Writer, "write");
        let mut run = AgentRun::from_spec("s", None, &spec, "F:/ws");
        run.output_artifact = Some("x".repeat(2000));
        let summary = AgentSummary::from_run(&run, Some("gpt-4o".into()), Some("working".into()));
        assert!(summary.output_preview.unwrap().chars().count() <= 601);
        assert_eq!(summary.model_name, "gpt-4o");
        assert_eq!(summary.archetype_label, "Writer");
        assert_eq!(summary.activity, "working");
    }

    #[test]
    fn summary_falls_back_to_model_tail() {
        let spec = AgentSpec::new("Coder", AgentArchetype::Coder, "code");
        let mut run = AgentRun::from_spec("s", None, &spec, "F:/ws");
        run.model_id = "prov:deepseek-chat".into();
        let summary = AgentSummary::from_run(&run, None, None);
        assert_eq!(summary.model_name, "deepseek-chat");
        assert!(summary.activity.is_empty());
    }
}
