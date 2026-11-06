//! Post-run bookkeeping: persistence, blackboard publication, git merging,
//! dependency wake-ups and session completion.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use reverie_core::domain::{AgentArchetype, AgentRun, AgentSpec, AgentStatus, SessionStatus, TaskCategory};
use reverie_core::events::SystemEvent;
use reverie_core::time;

use crate::agent_actor::AgentOutcome;
use crate::scheduler::{estimate_complexity, RuntimeError, RuntimeResult, SessionState, SwarmScheduler};
use crate::warden::{EnvironmentWardenAgent, MergeOutcome};

impl SwarmScheduler {
    /// Persist an agent's terminal state and cascade its effects.
    pub(crate) async fn finalize(
        self: &Arc<Self>,
        state: &Arc<SessionState>,
        run: &AgentRun,
        outcome: AgentOutcome,
    ) {
        let status = outcome.status;
        let error = outcome.error.clone();
        let output = outcome.output.clone();

        let _ = self
            .deps
            .store
            .finish_agent(&run.id, status, output.as_deref(), error.as_deref())
            .await;

        let elapsed = time::elapsed_ms(run.created_at, time::now());
        let _ = self
            .deps
            .store
            .record_model_outcome(&run.model_id, status == AgentStatus::Completed, elapsed)
            .await;

        if let Some(handle) = self.handle(&run.id) {
            handle.set_activity(match status {
                AgentStatus::Completed => "Completed".to_string(),
                AgentStatus::Failed => format!(
                    "Failed: {}",
                    reverie_core::util::truncate_chars(error.as_deref().unwrap_or("unknown error"), 120)
                ),
                AgentStatus::Cancelled => "Cancelled".to_string(),
                other => other.as_str().to_string(),
            });
        }

        self.deps.sink.emit(SystemEvent::AgentStatusChanged {
            session_id: run.session_id.clone(),
            agent_id: run.id.clone(),
            status: status.as_str().to_string(),
            error_message: error.clone(),
        });

        if status == AgentStatus::Completed {
            if let Some(text) = output.as_deref() {
                if !matches!(run.archetype, AgentArchetype::MetaOrchestrator) {
                    let summary = first_line(text, 220);
                    let tag = format!("result:{}", run.archetype.as_str());
                    let _ = self
                        .publish_finding(&run.session_id, &run.id, &tag, &summary, text)
                        .await;
                }
            }
            self.merge_if_branched(state, run).await;
        }

        state.completed.insert(run.id.clone(), ());
        state.started.remove(&run.id);

        self.wake_dependents(state).await;
        self.maybe_complete_session(state).await;
    }

    /// Merge an agent's git branch back into the session workspace.
    async fn merge_if_branched(self: &Arc<Self>, state: &Arc<SessionState>, run: &AgentRun) {
        let Some(branch) = run.git_branch.clone() else {
            return;
        };
        let workspace = state.config.read().workspace.clone();
        let outcome = EnvironmentWardenAgent::merge_branch(&workspace, &branch).await;

        match outcome {
            MergeOutcome::Clean | MergeOutcome::AlreadyUpToDate => {
                let worktree = std::path::PathBuf::from(&run.workspace_dir);
                if worktree != workspace {
                    EnvironmentWardenAgent::remove_worktree(&workspace, &worktree).await;
                }
                self.deps.sink.emit(SystemEvent::LogMessage {
                    level: "info".to_string(),
                    scope: run.role_name.clone(),
                    message: format!("branch `{branch}` merged cleanly"),
                });
            }
            MergeOutcome::Conflict { files } => {
                self.deps.sink.emit(SystemEvent::LogMessage {
                    level: "warn".to_string(),
                    scope: run.role_name.clone(),
                    message: format!("merge conflict on `{branch}`: {}", files.join(", ")),
                });
                let _ = self.spawn_merge_resolver(state, &branch, &files).await;
            }
            MergeOutcome::Failed(err) => {
                self.deps.sink.emit(SystemEvent::LogMessage {
                    level: "error".to_string(),
                    scope: run.role_name.clone(),
                    message: format!("merge of `{branch}` failed: {err}"),
                });
            }
        }
    }

    /// Spawn a Merge-Resolver agent for a conflicted branch.
    async fn spawn_merge_resolver(
        self: &Arc<Self>,
        state: &Arc<SessionState>,
        branch: &str,
        files: &[String],
    ) -> RuntimeResult<String> {
        let existing = self.deps.store.list_agents(&state.session_id).await?;
        let already_running = existing.iter().any(|agent| {
            agent.archetype == AgentArchetype::MergeResolver && !agent.status.is_terminal()
        });
        if already_running {
            return Err(RuntimeError::Invalid("a merge resolver is already running".to_string()));
        }

        let workspace = state.config.read().workspace.clone();
        let task = format!(
            "A `git merge --no-ff {branch}` in `{}` stopped with conflicts in these files:\n{}\n\n\
             Resolve every conflict by reading both sides, preserving the intent of each change, then \
             run `git add` on the resolved files and `git commit --no-edit` to complete the merge. \
             Verify the result still parses or builds before reporting success.",
            workspace.display(),
            files.iter().map(|file| format!("- {file}")).collect::<Vec<_>>().join("\n")
        );

        let spec = AgentSpec::new("Merge-Resolver", AgentArchetype::MergeResolver, task);
        let mut run = AgentRun::from_spec(&state.session_id, None, &spec, &workspace.to_string_lossy());
        run.status = AgentStatus::Queued;
        let complexity = estimate_complexity(&spec);
        self.spawn_one(state, run, None, TaskCategory::Debugging, complexity).await
    }

    /// Launch every waiting agent whose dependencies have all completed.
    async fn wake_dependents(self: &Arc<Self>, state: &Arc<SessionState>) {
        let completed = state.completed_ids();
        let Ok(runs) = self.deps.store.list_agents(&state.session_id).await else {
            return;
        };

        for run in runs {
            if run.status != AgentStatus::WaitingDependency {
                continue;
            }
            if state.started.contains_key(&run.id) {
                continue;
            }
            if !run.is_ready(&completed) {
                continue;
            }
            if self.deps.store.update_agent_status(&run.id, AgentStatus::Queued, None).await.is_err() {
                continue;
            }
            state.started.insert(run.id.clone(), ());
            if let Some(handle) = self.handle(&run.id) {
                handle.set_activity("Dependencies satisfied — starting");
            }
            let category = category_for(&run.archetype);
            let complexity = 0.5;
            let mut ready = run.clone();
            ready.status = AgentStatus::Queued;
            self.launch(state.clone(), ready, None, category, complexity);
        }
    }

    /// Close the session once every agent has settled.
    async fn maybe_complete_session(self: &Arc<Self>, state: &Arc<SessionState>) {
        if state.finished.load(Ordering::SeqCst) {
            return;
        }
        if self.has_pending_agents(&state.session_id).await {
            return;
        }

        let Ok(runs) = self.deps.store.list_agents(&state.session_id).await else {
            return;
        };
        if runs.is_empty() {
            return;
        }
        if state.finished.swap(true, Ordering::SeqCst) {
            return;
        }

        let failed = runs.iter().any(|run| run.status == AgentStatus::Failed);
        let cancelled = runs.iter().any(|run| run.status == AgentStatus::Cancelled);

        if failed || cancelled {
            let _ = self
                .deps
                .store
                .update_session_status(&state.session_id, SessionStatus::Failed)
                .await;
            self.deps.sink.emit(SystemEvent::SessionStatusChanged {
                session_id: state.session_id.clone(),
                status: SessionStatus::Failed.as_str().to_string(),
            });
            return;
        }

        let summary = runs
            .iter()
            .find(|run| run.archetype == AgentArchetype::MetaOrchestrator)
            .and_then(|run| run.output_artifact.clone());

        match summary {
            Some(summary) => {
                let _ = self.set_final_summary(&state.session_id, &summary).await;
            }
            None => {
                let _ = self
                    .deps
                    .store
                    .update_session_status(&state.session_id, SessionStatus::Completed)
                    .await;
                self.deps.sink.emit(SystemEvent::SessionStatusChanged {
                    session_id: state.session_id.clone(),
                    status: SessionStatus::Completed.as_str().to_string(),
                });
            }
        }
    }
}

/// Task category implied by an archetype, used when waking a dependency.
fn category_for(archetype: &AgentArchetype) -> TaskCategory {
    match archetype {
        AgentArchetype::MetaOrchestrator => TaskCategory::Orchestration,
        AgentArchetype::EnvironmentWarden | AgentArchetype::ContextBroker => TaskCategory::Architecture,
        AgentArchetype::Coder | AgentArchetype::MergeResolver => TaskCategory::Coding,
        AgentArchetype::Reviewer => TaskCategory::Debugging,
        AgentArchetype::Researcher => TaskCategory::Research,
        AgentArchetype::DataExtractor => TaskCategory::Extraction,
        AgentArchetype::VisionInspector => TaskCategory::VisionInspection,
        AgentArchetype::Analyst | AgentArchetype::Writer => TaskCategory::Summarization,
        AgentArchetype::Generalist => TaskCategory::General,
    }
}

/// First non-empty line, truncated — used as the blackboard summary.
fn first_line(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("completed");
    reverie_core::util::truncate_chars(line, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_line_skips_headings() {
        assert_eq!(first_line("# Title\n\nActual summary here\nmore", 100), "Actual summary here");
    }

    #[test]
    fn first_line_falls_back_when_empty() {
        assert_eq!(first_line("   \n\n", 100), "completed");
    }

    #[test]
    fn first_line_truncates() {
        let long = "a".repeat(500);
        assert!(first_line(&long, 50).chars().count() <= 51);
    }

    #[test]
    fn categories_follow_archetype() {
        assert_eq!(category_for(&AgentArchetype::Coder), TaskCategory::Coding);
        assert_eq!(category_for(&AgentArchetype::Researcher), TaskCategory::Research);
        assert_eq!(category_for(&AgentArchetype::Writer), TaskCategory::Summarization);
    }
}
