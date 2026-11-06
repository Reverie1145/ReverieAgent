//! The execution context handed to every tool invocation.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use reverie_core::domain::{AgentSpec, SearchEngineConfig};
use reverie_core::events::{EventSink, SystemEvent};
use reverie_store::Store;

use crate::diff::FileDiff;
use crate::error::{ToolError, ToolResult};
use crate::registry::GlobalToolRegistry;
use crate::vfs::VfsLockManager;

/// Default cap on how much tool output is fed back into a model's context.
pub const DEFAULT_MAX_OUTPUT_CHARS: usize = 60_000;

/// The bridge that lets orchestration tools reach the swarm scheduler without
/// the tool crate depending on the runtime crate.
#[async_trait]
pub trait OrchestrationBridge: Send + Sync + 'static {
    /// Persist the swarm topology chosen by the Meta-Orchestrator.
    async fn configure_strategy(
        &self,
        session_id: &str,
        topology: &str,
        context_strategy: &str,
        isolation: &str,
    ) -> ToolResult<String>;

    /// Spawn a batch of sub-agents and return their ids.
    async fn spawn_sub_agents(
        &self,
        session_id: &str,
        parent_agent_id: &str,
        specs: Vec<AgentSpec>,
    ) -> ToolResult<Vec<String>>;

    /// Record the session's final deliverable and close it out.
    async fn finalize_session(&self, session_id: &str, agent_id: &str, summary: &str) -> ToolResult<()>;

    /// Human-readable snapshot of the current swarm.
    async fn swarm_status(&self, session_id: &str) -> ToolResult<String>;
}

/// Everything a tool needs to do its job.
#[derive(Clone)]
pub struct ToolExecutionContext {
    pub session_id: String,
    pub agent_id: String,
    pub role_name: String,
    pub workspace_dir: PathBuf,
    pub store: Arc<Store>,
    pub vfs: Arc<VfsLockManager>,
    pub sink: Arc<dyn EventSink>,
    pub http: reqwest::Client,
    pub search_config: SearchEngineConfig,
    pub web_search_enabled: bool,
    pub registry: Arc<GlobalToolRegistry>,
    pub orchestration: Option<Arc<dyn OrchestrationBridge>>,
    pub max_output_chars: usize,
}

impl std::fmt::Debug for ToolExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolExecutionContext")
            .field("session_id", &self.session_id)
            .field("agent_id", &self.agent_id)
            .field("role_name", &self.role_name)
            .field("workspace_dir", &self.workspace_dir)
            .field("web_search_enabled", &self.web_search_enabled)
            .finish()
    }
}

impl ToolExecutionContext {
    /// Lexically normalize a path (collapses `.` and resolves `..` without
    /// touching the filesystem, so it works for not-yet-created files).
    pub fn normalize(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !out.pop() {
                        out.push("..");
                    }
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    /// Resolve a possibly relative path against the agent's workspace.
    pub fn resolve(&self, raw: &str) -> PathBuf {
        let candidate = Path::new(raw);
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.workspace_dir.join(candidate)
        };
        Self::normalize(&joined)
    }

    /// Resolve a path that will be written, refusing anything outside the
    /// agent's workspace.
    pub fn resolve_writable(&self, raw: &str) -> ToolResult<PathBuf> {
        let path = self.resolve(raw);
        let root = Self::normalize(&self.workspace_dir);
        if !path.starts_with(&root) {
            return Err(ToolError::Denied(format!(
                "write outside the agent workspace is not permitted: {}",
                path.display()
            )));
        }
        Ok(path)
    }

    /// Ensure the parent directory exists for a writable path.
    pub fn ensure_parent(&self, path: &Path) -> ToolResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    /// Path to the per-session runtime scratch directory.
    pub fn runtime_dir(&self) -> PathBuf {
        self.workspace_dir.join(reverie_core::RUNTIME_DIR)
    }

    /// Path to the custom tool script directory.
    pub fn custom_tools_dir(&self) -> PathBuf {
        self.runtime_dir().join("custom_tools")
    }

    /// Trim oversized tool output so a single call cannot blow the window.
    pub fn cap_output(&self, text: String) -> String {
        let limit = if self.max_output_chars == 0 { DEFAULT_MAX_OUTPUT_CHARS } else { self.max_output_chars };
        reverie_core::util::truncate_middle(&text, limit)
    }

    /// Broadcast a one-line activity description for the agent card.
    pub fn emit_activity(&self, activity: impl Into<String>) {
        self.sink.emit(SystemEvent::AgentActivity {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            activity: activity.into(),
        });
    }

    /// Broadcast a workspace diff.
    pub fn emit_diff(&self, diff: &FileDiff) {
        if diff.is_empty() && !diff.is_new {
            return;
        }
        self.sink.emit(SystemEvent::WorkspaceDiffUpdated {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            file_path: diff.path.clone(),
            diff: diff.diff.clone(),
        });
    }

    /// Broadcast a diagnostic log line.
    pub fn log(&self, level: &str, message: impl Into<String>) {
        self.sink.emit(SystemEvent::LogMessage {
            level: level.to_string(),
            scope: self.role_name.clone(),
            message: message.into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_resolves_dot_segments() {
        assert_eq!(
            ToolExecutionContext::normalize(Path::new("/a/./b/../c/file.rs")),
            PathBuf::from("/a/c/file.rs")
        );
    }

    #[test]
    fn leading_parent_dirs_are_preserved() {
        assert_eq!(ToolExecutionContext::normalize(Path::new("../x")), PathBuf::from("../x"));
    }
}
