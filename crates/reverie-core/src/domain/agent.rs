//! Sub-agent execution records and spawn specifications.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};
use crate::llm::ThinkingLevel;
use crate::time;

/// Lifecycle state of one agent run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Queued,
    Running,
    WaitingDependency,
    Completed,
    Failed,
    Cancelled,
}

impl AgentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Queued => "queued",
            AgentStatus::Running => "running",
            AgentStatus::WaitingDependency => "waiting_dependency",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed => "failed",
            AgentStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "queued" => Ok(AgentStatus::Queued),
            "running" => Ok(AgentStatus::Running),
            "waiting_dependency" | "waiting" => Ok(AgentStatus::WaitingDependency),
            "completed" => Ok(AgentStatus::Completed),
            "failed" => Ok(AgentStatus::Failed),
            "cancelled" | "canceled" => Ok(AgentStatus::Cancelled),
            other => Err(CoreError::InvalidEnum { kind: "AgentStatus", value: other.to_string() }),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled)
    }

    pub fn is_active(self) -> bool {
        matches!(self, AgentStatus::Running)
    }
}

/// Role archetype driving the system prompt and the default model choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentArchetype {
    MetaOrchestrator,
    ContextBroker,
    EnvironmentWarden,
    Researcher,
    Coder,
    Reviewer,
    Analyst,
    Writer,
    VisionInspector,
    DataExtractor,
    MergeResolver,
    Generalist,
}

impl AgentArchetype {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentArchetype::MetaOrchestrator => "meta_orchestrator",
            AgentArchetype::ContextBroker => "context_broker",
            AgentArchetype::EnvironmentWarden => "environment_warden",
            AgentArchetype::Researcher => "researcher",
            AgentArchetype::Coder => "coder",
            AgentArchetype::Reviewer => "reviewer",
            AgentArchetype::Analyst => "analyst",
            AgentArchetype::Writer => "writer",
            AgentArchetype::VisionInspector => "vision_inspector",
            AgentArchetype::DataExtractor => "data_extractor",
            AgentArchetype::MergeResolver => "merge_resolver",
            AgentArchetype::Generalist => "generalist",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "meta_orchestrator" | "orchestrator" => Ok(AgentArchetype::MetaOrchestrator),
            "context_broker" => Ok(AgentArchetype::ContextBroker),
            "environment_warden" | "warden" => Ok(AgentArchetype::EnvironmentWarden),
            "researcher" | "search" => Ok(AgentArchetype::Researcher),
            "coder" | "developer" | "dev" => Ok(AgentArchetype::Coder),
            "reviewer" | "qa" => Ok(AgentArchetype::Reviewer),
            "analyst" => Ok(AgentArchetype::Analyst),
            "writer" | "author" => Ok(AgentArchetype::Writer),
            "vision_inspector" | "vision" => Ok(AgentArchetype::VisionInspector),
            "data_extractor" | "extractor" => Ok(AgentArchetype::DataExtractor),
            "merge_resolver" | "merger" => Ok(AgentArchetype::MergeResolver),
            "generalist" | "general" => Ok(AgentArchetype::Generalist),
            other => Err(CoreError::InvalidEnum { kind: "AgentArchetype", value: other.to_string() }),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            AgentArchetype::MetaOrchestrator => "Meta-Orchestrator",
            AgentArchetype::ContextBroker => "Context-Broker",
            AgentArchetype::EnvironmentWarden => "Environment-Warden",
            AgentArchetype::Researcher => "Researcher",
            AgentArchetype::Coder => "Coder",
            AgentArchetype::Reviewer => "Reviewer",
            AgentArchetype::Analyst => "Analyst",
            AgentArchetype::Writer => "Writer",
            AgentArchetype::VisionInspector => "Vision-Inspector",
            AgentArchetype::DataExtractor => "Data-Extractor",
            AgentArchetype::MergeResolver => "Merge-Resolver",
            AgentArchetype::Generalist => "Generalist",
        }
    }

    /// Stable key used by `evolution_store.key_name` for the role prompt.
    pub fn prompt_key(&self) -> String {
        format!("archetype:{}", self.as_str())
    }

    /// Whether this archetype benefits from a large context window.
    pub fn needs_long_context(&self) -> bool {
        matches!(self, AgentArchetype::MetaOrchestrator | AgentArchetype::Reviewer | AgentArchetype::Analyst)
    }
}

/// One persisted agent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub id: String,
    pub session_id: String,
    pub parent_agent_id: Option<String>,
    pub role_name: String,
    pub archetype: AgentArchetype,
    pub model_id: String,
    pub thinking_level: ThinkingLevel,
    pub task_description: String,
    pub depend_on_agent_ids: Vec<String>,
    pub attached_file_ids: Vec<String>,
    pub workspace_dir: String,
    pub git_branch: Option<String>,
    pub status: AgentStatus,
    pub message_history_json: String,
    pub output_artifact: Option<String>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub reasoning_tokens: i64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl AgentRun {
    pub fn from_spec(session_id: &str, parent_agent_id: Option<&str>, spec: &AgentSpec, workspace_dir: &str) -> Self {
        AgentRun {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            parent_agent_id: parent_agent_id.map(|v| v.to_string()),
            role_name: spec.role_name.clone(),
            archetype: spec.archetype.clone(),
            model_id: spec.model_id.clone().unwrap_or_default(),
            thinking_level: spec.thinking_level.unwrap_or(ThinkingLevel::Medium),
            task_description: spec.task_description.clone(),
            depend_on_agent_ids: spec.depend_on_agent_ids.clone(),
            attached_file_ids: spec.attached_file_ids.clone(),
            workspace_dir: spec.workspace_dir.clone().unwrap_or_else(|| workspace_dir.to_string()),
            git_branch: None,
            status: if spec.depend_on_agent_ids.is_empty() {
                AgentStatus::Queued
            } else {
                AgentStatus::WaitingDependency
            },
            message_history_json: "[]".to_string(),
            output_artifact: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            reasoning_tokens: 0,
            error_message: None,
            created_at: time::now(),
            finished_at: None,
        }
    }

    pub fn elapsed_ms(&self) -> i64 {
        match self.finished_at {
            Some(end) => time::elapsed_ms(self.created_at, end),
            None => time::elapsed_ms(self.created_at, time::now()),
        }
    }

    pub fn total_tokens(&self) -> i64 {
        self.prompt_tokens + self.completion_tokens
    }

    /// True when the run has no upstream dependencies left to satisfy.
    pub fn is_ready(&self, completed: &[String]) -> bool {
        self.depend_on_agent_ids.iter().all(|dep| completed.iter().any(|c| c == dep))
    }
}

/// Instruction set for spawning one sub-agent. Produced by the Meta-Orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub role_name: String,
    pub archetype: AgentArchetype,
    /// `None` lets the router pick.
    #[serde(default)]
    pub model_id: Option<String>,
    /// `None` lets the router pick based on archetype complexity.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    pub task_description: String,
    #[serde(default)]
    pub depend_on_agent_ids: Vec<String>,
    #[serde(default)]
    pub attached_file_ids: Vec<String>,
    /// `None` = every tool allowed.
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    /// `None` = inherit the session workspace.
    #[serde(default)]
    pub workspace_dir: Option<String>,
    /// Hint for the router; `None` derives it from the archetype.
    #[serde(default)]
    pub task_category: Option<crate::domain::model::TaskCategory>,
}

impl AgentSpec {
    pub fn new(role_name: impl Into<String>, archetype: AgentArchetype, task_description: impl Into<String>) -> Self {
        AgentSpec {
            role_name: role_name.into(),
            archetype,
            model_id: None,
            thinking_level: None,
            task_description: task_description.into(),
            depend_on_agent_ids: Vec::new(),
            attached_file_ids: Vec::new(),
            allowed_tools: None,
            workspace_dir: None,
            task_category: None,
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        if self.role_name.trim().is_empty() {
            return Err(CoreError::Validation("role_name must not be empty".into()));
        }
        if self.task_description.trim().is_empty() {
            return Err(CoreError::Validation("task_description must not be empty".into()));
        }
        Ok(())
    }

    /// The task category the router should use.
    pub fn effective_category(&self) -> crate::domain::model::TaskCategory {
        use crate::domain::model::TaskCategory;
        if let Some(cat) = self.task_category {
            return cat;
        }
        match self.archetype {
            AgentArchetype::MetaOrchestrator => TaskCategory::Orchestration,
            AgentArchetype::EnvironmentWarden | AgentArchetype::ContextBroker => TaskCategory::Architecture,
            AgentArchetype::Coder | AgentArchetype::MergeResolver => TaskCategory::Coding,
            AgentArchetype::Reviewer => TaskCategory::Debugging,
            AgentArchetype::Researcher => TaskCategory::Research,
            AgentArchetype::DataExtractor => TaskCategory::Extraction,
            AgentArchetype::VisionInspector => TaskCategory::VisionInspection,
            AgentArchetype::Analyst => TaskCategory::Summarization,
            AgentArchetype::Writer => TaskCategory::Summarization,
            AgentArchetype::Generalist => TaskCategory::General,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_roundtrip() {
        for s in [
            AgentStatus::Queued,
            AgentStatus::Running,
            AgentStatus::WaitingDependency,
            AgentStatus::Completed,
            AgentStatus::Failed,
            AgentStatus::Cancelled,
        ] {
            assert_eq!(AgentStatus::parse(s.as_str()).unwrap(), s);
        }
        assert!(AgentStatus::Failed.is_terminal());
        assert!(!AgentStatus::Running.is_terminal());
    }

    #[test]
    fn archetype_aliases() {
        assert_eq!(AgentArchetype::parse("Dev").unwrap(), AgentArchetype::Coder);
        assert_eq!(AgentArchetype::parse("meta-orchestrator").unwrap(), AgentArchetype::MetaOrchestrator);
        assert!(AgentArchetype::parse("wizard").is_err());
        assert_eq!(AgentArchetype::Coder.prompt_key(), "archetype:coder");
    }

    #[test]
    fn spec_derives_category_from_archetype() {
        let spec = AgentSpec::new("Researcher 1", AgentArchetype::Researcher, "find sources");
        assert_eq!(spec.effective_category(), crate::domain::model::TaskCategory::Research);
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn dependency_readiness() {
        let mut spec = AgentSpec::new("a", AgentArchetype::Coder, "t");
        spec.depend_on_agent_ids = vec!["x".into(), "y".into()];
        let run = AgentRun::from_spec("s", None, &spec, "F:/ws");
        assert_eq!(run.status, AgentStatus::WaitingDependency);
        assert!(!run.is_ready(&["x".to_string()]));
        assert!(run.is_ready(&["x".to_string(), "y".to_string()]));
    }

    #[test]
    fn no_deps_means_queued() {
        let spec = AgentSpec::new("a", AgentArchetype::Writer, "t");
        let run = AgentRun::from_spec("s", Some("p"), &spec, "F:/ws");
        assert_eq!(run.status, AgentStatus::Queued);
        assert!(run.is_ready(&[]));
        assert_eq!(run.parent_agent_id.as_deref(), Some("p"));
    }
}
