//! Task sessions: one user objective bound to one workspace.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};
use crate::llm::ThinkingLevel;
use crate::time;

/// Lifecycle state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Planning,
    Running,
    Paused,
    Completed,
    Failed,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Planning => "planning",
            SessionStatus::Running => "running",
            SessionStatus::Paused => "paused",
            SessionStatus::Completed => "completed",
            SessionStatus::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "planning" => Ok(SessionStatus::Planning),
            "running" => Ok(SessionStatus::Running),
            "paused" => Ok(SessionStatus::Paused),
            "completed" => Ok(SessionStatus::Completed),
            "failed" => Ok(SessionStatus::Failed),
            other => Err(CoreError::InvalidEnum { kind: "SessionStatus", value: other.to_string() }),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, SessionStatus::Completed | SessionStatus::Failed)
    }
}

/// How the swarm is wired together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyMode {
    FlatSwarm,
    HierarchicalTree,
    DagPipeline,
}

impl TopologyMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TopologyMode::FlatSwarm => "flat_swarm",
            TopologyMode::HierarchicalTree => "hierarchical_tree",
            TopologyMode::DagPipeline => "dag_pipeline",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "flat_swarm" | "flat" | "swarm" => Ok(TopologyMode::FlatSwarm),
            "hierarchical_tree" | "hierarchical" | "tree" => Ok(TopologyMode::HierarchicalTree),
            "dag_pipeline" | "dag" | "pipeline" => Ok(TopologyMode::DagPipeline),
            other => Err(CoreError::InvalidEnum { kind: "TopologyMode", value: other.to_string() }),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TopologyMode::FlatSwarm => "Flat swarm",
            TopologyMode::HierarchicalTree => "Hierarchical tree",
            TopologyMode::DagPipeline => "DAG pipeline",
        }
    }
}

/// How findings travel between agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextStrategy {
    Blackboard,
    PubSub,
    PeerToPeer,
    Hybrid,
}

impl ContextStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextStrategy::Blackboard => "blackboard",
            ContextStrategy::PubSub => "pub_sub",
            ContextStrategy::PeerToPeer => "peer_to_peer",
            ContextStrategy::Hybrid => "hybrid",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "blackboard" => Ok(ContextStrategy::Blackboard),
            "pub_sub" | "pubsub" => Ok(ContextStrategy::PubSub),
            "peer_to_peer" | "p2p" | "peer" => Ok(ContextStrategy::PeerToPeer),
            "hybrid" => Ok(ContextStrategy::Hybrid),
            other => Err(CoreError::InvalidEnum { kind: "ContextStrategy", value: other.to_string() }),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ContextStrategy::Blackboard => "Blackboard",
            ContextStrategy::PubSub => "Pub/Sub",
            ContextStrategy::PeerToPeer => "Peer to peer",
            ContextStrategy::Hybrid => "Hybrid",
        }
    }

    /// Whether agents should actively push summaries onto the blackboard.
    pub fn uses_blackboard(self) -> bool {
        matches!(self, ContextStrategy::Blackboard | ContextStrategy::Hybrid | ContextStrategy::PubSub)
    }
}

/// How workspace writes are isolated between agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationStrategy {
    SharedVfsLock,
    GitWorktree,
    SandboxDir,
}

impl IsolationStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            IsolationStrategy::SharedVfsLock => "shared_vfs_lock",
            IsolationStrategy::GitWorktree => "git_worktree",
            IsolationStrategy::SandboxDir => "sandbox_dir",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "shared_vfs_lock" | "shared" => Ok(IsolationStrategy::SharedVfsLock),
            "git_worktree" | "worktree" | "git" => Ok(IsolationStrategy::GitWorktree),
            "sandbox_dir" | "sandbox" => Ok(IsolationStrategy::SandboxDir),
            other => Err(CoreError::InvalidEnum { kind: "IsolationStrategy", value: other.to_string() }),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IsolationStrategy::SharedVfsLock => "Shared VFS lock",
            IsolationStrategy::GitWorktree => "Git worktree",
            IsolationStrategy::SandboxDir => "Sandbox dir",
        }
    }
}

/// A user task session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub workspace_path: String,
    pub user_objective: String,
    /// `auto` or a `ThinkingLevel` name.
    pub thinking_mode_override: String,
    pub web_search_enabled: bool,
    pub status: SessionStatus,
    pub topology_mode: TopologyMode,
    pub context_strategy: ContextStrategy,
    pub isolation_strategy: IsolationStrategy,
    pub final_summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn create(
        title: impl Into<String>,
        workspace_path: impl Into<String>,
        user_objective: impl Into<String>,
        thinking_mode_override: impl Into<String>,
        web_search_enabled: bool,
    ) -> Self {
        let now = time::now();
        Session {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            workspace_path: workspace_path.into(),
            user_objective: user_objective.into(),
            thinking_mode_override: thinking_mode_override.into(),
            web_search_enabled,
            status: SessionStatus::Planning,
            topology_mode: TopologyMode::HierarchicalTree,
            context_strategy: ContextStrategy::Blackboard,
            isolation_strategy: IsolationStrategy::SharedVfsLock,
            final_summary: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Resolved thinking override: `None` means "let the orchestrator decide".
    pub fn thinking_override(&self) -> Option<ThinkingLevel> {
        ThinkingLevel::parse_optional(&self.thinking_mode_override).unwrap_or(None)
    }

    /// Normalize and validate the override string for persistence.
    pub fn normalize_override(raw: &str) -> CoreResult<String> {
        match ThinkingLevel::parse_optional(raw)? {
            Some(level) => Ok(level.as_str().to_string()),
            None => Ok("auto".to_string()),
        }
    }

    pub fn validate(&self) -> CoreResult<()> {
        if self.title.trim().is_empty() {
            return Err(CoreError::Validation("session title must not be empty".into()));
        }
        if self.user_objective.trim().is_empty() {
            return Err(CoreError::Validation("user objective must not be empty".into()));
        }
        if self.workspace_path.trim().is_empty() {
            return Err(CoreError::Validation("workspace path must not be empty".into()));
        }
        Ok(())
    }

    /// Absolute path of the runtime scratch directory for this session.
    pub fn runtime_dir(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.workspace_path).join(crate::RUNTIME_DIR)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session::create("Build a thing", "F:/tmp/ws", "do it", "auto", true)
    }

    #[test]
    fn create_defaults_are_sane() {
        let s = sample();
        assert_eq!(s.status, SessionStatus::Planning);
        assert_eq!(s.topology_mode, TopologyMode::HierarchicalTree);
        assert!(s.thinking_override().is_none());
        assert!(s.validate().is_ok());
    }

    #[test]
    fn override_normalization() {
        assert_eq!(Session::normalize_override("AUTO").unwrap(), "auto");
        assert_eq!(Session::normalize_override("high").unwrap(), "high");
        assert!(Session::normalize_override("ultra").is_err());
    }

    #[test]
    fn enum_roundtrips() {
        for status in [
            SessionStatus::Planning,
            SessionStatus::Running,
            SessionStatus::Paused,
            SessionStatus::Completed,
            SessionStatus::Failed,
        ] {
            assert_eq!(SessionStatus::parse(status.as_str()).unwrap(), status);
        }
        for topo in [TopologyMode::FlatSwarm, TopologyMode::HierarchicalTree, TopologyMode::DagPipeline] {
            assert_eq!(TopologyMode::parse(topo.as_str()).unwrap(), topo);
        }
        for ctx in [ContextStrategy::Blackboard, ContextStrategy::PubSub, ContextStrategy::PeerToPeer, ContextStrategy::Hybrid] {
            assert_eq!(ContextStrategy::parse(ctx.as_str()).unwrap(), ctx);
        }
        for iso in [IsolationStrategy::SharedVfsLock, IsolationStrategy::GitWorktree, IsolationStrategy::SandboxDir] {
            assert_eq!(IsolationStrategy::parse(iso.as_str()).unwrap(), iso);
        }
    }

    #[test]
    fn runtime_dir_appends_hidden_folder() {
        let s = sample();
        assert!(s.runtime_dir().ends_with(".reverie"));
    }
}
