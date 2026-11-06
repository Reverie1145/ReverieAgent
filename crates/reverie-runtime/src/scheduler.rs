//! The unbounded Tokio actor scheduler.
//!
//! One task per agent, a semaphore to bound in-flight model calls, and a DAG
//! dependency table so `waiting_dependency` agents wake the moment their
//! upstream finishes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use parking_lot::RwLock;
use reverie_core::domain::{
    AgentArchetype, AgentRun, AgentSpec, AgentStatus, BlackboardEntry, ContextStrategy,
    IsolationStrategy, Session, SessionStatus, TaskCategory, TopologyMode,
};
use reverie_core::events::SystemEvent;
use reverie_core::llm::ThinkingLevel;
use reverie_store::StoreError;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use crate::agent_actor::{run_agent, ActorDeps, ActorInput};
use crate::context_broker::{BrokerRequest, ContextBrokerAgent};
use crate::control::AgentControl;
use crate::prompt::build_system_prompt;
use crate::snapshot::{AgentDetail, AgentSummary, SessionSnapshot, TokenTotals};

/// Errors raised by the runtime.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("store failure: {0}")]
    Store(#[from] StoreError),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("agent not found: {0}")]
    AgentNotFound(String),

    #[error("invalid request: {0}")]
    Invalid(String),

    #[error("{0}")]
    Other(String),
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;

/// Live configuration of a running session.
#[derive(Debug, Clone)]
pub struct SessionRunConfig {
    pub title: String,
    pub objective: String,
    pub workspace: PathBuf,
    pub web_search_enabled: bool,
    pub thinking_override: Option<ThinkingLevel>,
    pub topology: TopologyMode,
    pub context_strategy: ContextStrategy,
    pub isolation: IsolationStrategy,
}

impl SessionRunConfig {
    pub fn from_session(session: &Session) -> Self {
        SessionRunConfig {
            title: session.title.clone(),
            objective: session.user_objective.clone(),
            workspace: PathBuf::from(&session.workspace_path),
            web_search_enabled: session.web_search_enabled,
            thinking_override: session.thinking_override(),
            topology: session.topology_mode,
            context_strategy: session.context_strategy,
            isolation: session.isolation_strategy,
        }
    }
}

/// Per-agent handle kept in memory for live control.
#[derive(Debug)]
pub struct AgentHandle {
    pub agent_id: String,
    pub role_name: String,
    pub control: Arc<AgentControl>,
    pub activity: Arc<RwLock<String>>,
}

impl AgentHandle {
    fn new(run: &AgentRun) -> Self {
        AgentHandle {
            agent_id: run.id.clone(),
            role_name: run.role_name.clone(),
            control: Arc::new(AgentControl::new(run.model_id.clone(), run.thinking_level)),
            activity: Arc::new(RwLock::new("Queued".to_string())),
        }
    }

    pub fn set_activity(&self, text: impl Into<String>) {
        *self.activity.write() = text.into();
    }

    pub fn activity(&self) -> String {
        self.activity.read().clone()
    }
}

/// Everything about one live session.
pub struct SessionState {
    pub session_id: String,
    pub cancel: CancellationToken,
    pub paused: Arc<AtomicBool>,
    pub agents: DashMap<String, Arc<AgentHandle>>,
    pub completed: DashMap<String, ()>,
    pub started: DashMap<String, ()>,
    pub running: AtomicUsize,
    pub config: RwLock<SessionRunConfig>,
    pub final_summary: RwLock<Option<String>>,
    pub finished: AtomicBool,
    /// Watches the workspace for edits made outside the tool pipeline.
    pub watcher: parking_lot::Mutex<Option<crate::watcher::WorkspaceWatcher>>,
}

impl SessionState {
    fn new(session: &Session) -> Self {
        SessionState {
            session_id: session.id.clone(),
            cancel: CancellationToken::new(),
            paused: Arc::new(AtomicBool::new(false)),
            agents: DashMap::new(),
            completed: DashMap::new(),
            started: DashMap::new(),
            running: AtomicUsize::new(0),
            config: RwLock::new(SessionRunConfig::from_session(session)),
            final_summary: RwLock::new(None),
            finished: AtomicBool::new(false),
            watcher: parking_lot::Mutex::new(None),
        }
    }

    pub fn completed_ids(&self) -> Vec<String> {
        self.completed.iter().map(|entry| entry.key().clone()).collect()
    }
}

/// The scheduler itself.
pub struct SwarmScheduler {
    pub deps: Arc<ActorDeps>,
    sessions: DashMap<String, Arc<SessionState>>,
    handles: DashMap<String, Arc<AgentHandle>>,
    permits: Arc<Semaphore>,
}

/// Estimate how hard an agent's task is, for routing.
pub fn estimate_complexity(spec: &AgentSpec) -> f32 {
    let base = match spec.archetype {
        AgentArchetype::MetaOrchestrator => 0.95,
        AgentArchetype::EnvironmentWarden | AgentArchetype::ContextBroker => 0.7,
        AgentArchetype::MergeResolver => 0.75,
        AgentArchetype::Reviewer | AgentArchetype::Analyst => 0.65,
        AgentArchetype::Coder => 0.6,
        AgentArchetype::Researcher => 0.45,
        AgentArchetype::VisionInspector => 0.5,
        AgentArchetype::DataExtractor => 0.15,
        AgentArchetype::Writer | AgentArchetype::Generalist => 0.4,
    };
    let length_factor = (spec.task_description.chars().count() as f32 / 1500.0).min(0.3);
    (base + length_factor).clamp(0.05, 1.0)
}

impl SwarmScheduler {
    pub fn new(deps: Arc<ActorDeps>, max_concurrent: usize) -> Arc<Self> {
        Arc::new(SwarmScheduler {
            deps,
            sessions: DashMap::new(),
            handles: DashMap::new(),
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
        })
    }

    pub fn session_state(&self, session_id: &str) -> Option<Arc<SessionState>> {
        self.sessions.get(session_id).map(|entry| entry.clone())
    }

    pub fn handle(&self, agent_id: &str) -> Option<Arc<AgentHandle>> {
        self.handles.get(agent_id).map(|entry| entry.clone())
    }

    fn ensure_state(&self, session: &Session) -> Arc<SessionState> {
        if let Some(existing) = self.sessions.get(&session.id) {
            return existing.clone();
        }
        let state = Arc::new(SessionState::new(session));
        self.sessions.insert(session.id.clone(), state.clone());
        state
    }

    /// Create the root Meta-Orchestrator and start the swarm.
    pub async fn start_session(
        self: &Arc<Self>,
        session: &Session,
        orchestrator_model_id: Option<String>,
    ) -> RuntimeResult<()> {
        let state = self.ensure_state(session);

        // Watch the workspace so external edits surface in the UI.
        let workspace = state.config.read().workspace.clone();
        let watcher = crate::watcher::WorkspaceWatcher::start(
            &workspace,
            session.id.clone(),
            self.deps.sink.clone(),
        );
        *state.watcher.lock() = watcher;

        let spec = AgentSpec::new(
            "Meta-Orchestrator",
            AgentArchetype::MetaOrchestrator,
            session.user_objective.clone(),
        );
        let mut run = AgentRun::from_spec(&session.id, None, &spec, &session.workspace_path);
        run.status = AgentStatus::Queued;
        if let Some(model_id) = orchestrator_model_id.filter(|value| !value.trim().is_empty()) {
            run.model_id = model_id;
        }
        run.thinking_level = state
            .config
            .read()
            .thinking_override
            .unwrap_or(ThinkingLevel::High);

        let agent_id = self.spawn_one(&state, run, None, TaskCategory::Orchestration, 0.95).await?;
        self.deps.sink.emit(SystemEvent::SessionStatusChanged {
            session_id: session.id.clone(),
            status: SessionStatus::Running.as_str().to_string(),
        });
        let _ = agent_id;
        Ok(())
    }

    /// Persist, register and launch one agent.
    pub async fn spawn_one(
        self: &Arc<Self>,
        state: &Arc<SessionState>,
        run: AgentRun,
        allowed_tools: Option<Vec<String>>,
        category: TaskCategory,
        complexity: f32,
    ) -> RuntimeResult<String> {
        self.deps.store.insert_agent(&run).await?;
        let handle = Arc::new(AgentHandle::new(&run));
        self.handles.insert(run.id.clone(), handle.clone());
        state.agents.insert(run.id.clone(), handle.clone());

        self.deps.sink.emit(SystemEvent::AgentCreated {
            session_id: run.session_id.clone(),
            agent_id: run.id.clone(),
            role_name: run.role_name.clone(),
            archetype: run.archetype.as_str().to_string(),
            model_id: run.model_id.clone(),
            thinking_level: run.thinking_level.as_str().to_string(),
            status: run.status.as_str().to_string(),
        });

        let id = run.id.clone();
        if run.status == AgentStatus::WaitingDependency {
            handle.set_activity("Waiting for dependencies");
            return Ok(id);
        }
        state.started.insert(id.clone(), ());
        self.launch(state.clone(), run, allowed_tools, category, complexity);
        Ok(id)
    }

    /// Spawn a batch produced by the Meta-Orchestrator.
    pub async fn spawn_batch(
        self: &Arc<Self>,
        session_id: &str,
        parent_agent_id: &str,
        specs: Vec<AgentSpec>,
    ) -> RuntimeResult<Vec<String>> {
        let state = self
            .session_state(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;
        let session = self
            .deps
            .store
            .get_session(session_id)
            .await?
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;
        let isolation = state.config.read().isolation;

        let mut ids = Vec::with_capacity(specs.len());
        for spec in specs {
            let mut run = AgentRun::from_spec(session_id, Some(parent_agent_id), &spec, &session.workspace_path);

            let provisioned =
                crate::warden::EnvironmentWardenAgent::provision(&session, &run.id, &spec.role_name, isolation)
                    .await;
            run.workspace_dir = provisioned.dir.to_string_lossy().to_string();
            run.git_branch = provisioned.branch.clone();
            if provisioned.degraded && isolation == IsolationStrategy::GitWorktree {
                self.deps.sink.emit(SystemEvent::LogMessage {
                    level: "warn".to_string(),
                    scope: spec.role_name.clone(),
                    message: "git worktree isolation unavailable; falling back to the shared workspace"
                        .to_string(),
                });
            }

            if let Some(model_id) = &spec.model_id {
                run.model_id = model_id.clone();
            }
            let category = spec.effective_category();
            let complexity = estimate_complexity(&spec);
            run.thinking_level = match spec.thinking_level {
                Some(level) => level,
                // Let the routing policy pick the depth: max for planning,
                // off for high-throughput extraction work.
                None => reverie_llm::ModelRouter::default_thinking(category, complexity),
            };
            let id = self
                .spawn_one(&state, run, spec.allowed_tools.clone(), category, complexity)
                .await?;
            ids.push(id);
        }
        Ok(ids)
    }

    /// Kick off the actor task.
    pub fn launch(
        self: &Arc<Self>,
        state: Arc<SessionState>,
        run: AgentRun,
        allowed_tools: Option<Vec<String>>,
        category: TaskCategory,
        complexity: f32,
    ) {
        let scheduler = self.clone();
        tokio::spawn(async move {
            scheduler.run_agent_task(state, run, allowed_tools, category, complexity).await;
        });
    }

    async fn run_agent_task(
        self: Arc<Self>,
        state: Arc<SessionState>,
        run: AgentRun,
        allowed_tools: Option<Vec<String>>,
        category: TaskCategory,
        complexity: f32,
    ) {
        let permit = match self.permits.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        state.running.fetch_add(1, Ordering::SeqCst);

        let agent_id = run.id.clone();
        let handle = self.handle(&agent_id);
        if let Some(handle) = &handle {
            handle.set_activity("Starting");
        }
        let _ = self
            .deps
            .store
            .update_agent_status(&agent_id, AgentStatus::Running, None)
            .await;
        self.deps.sink.emit(SystemEvent::AgentStatusChanged {
            session_id: run.session_id.clone(),
            agent_id: agent_id.clone(),
            status: AgentStatus::Running.as_str().to_string(),
            error_message: None,
        });

        let session = match self.deps.store.get_session(&run.session_id).await {
            Ok(Some(session)) => session,
            _ => {
                state.running.fetch_sub(1, Ordering::SeqCst);
                drop(permit);
                return;
            }
        };

        let evolved = self
            .deps
            .store
            .archetype_prompt(run.archetype.as_str())
            .await
            .ok()
            .flatten();
        let system_prompt = build_system_prompt(
            &run.archetype,
            evolved.as_deref(),
            &session,
            &run.role_name,
            &run.workspace_dir,
            state.config.read().web_search_enabled,
        );

        let briefing = {
            let config = state.config.read().clone();
            let budget = ContextBrokerAgent::context_budget(32_768, run.thinking_level.context_share());
            match ContextBrokerAgent::build_context(
                &self.deps.store,
                BrokerRequest { session: &session, run: &run, strategy: config.context_strategy, max_chars: budget },
            )
            .await
            {
                Ok(text) => text,
                Err(err) => format!("(briefing unavailable: {err})\n\n## Your task\n{}", run.task_description),
            }
        };

        if let Some(handle) = &handle {
            handle.set_activity("Reading briefing");
        }

        let control = handle
            .as_ref()
            .map(|handle| handle.control.clone())
            .unwrap_or_else(|| Arc::new(AgentControl::new(run.model_id.clone(), run.thinking_level)));

        let web_search_enabled = state.config.read().web_search_enabled;
        let search_config = self.search_config().await;

        let input = ActorInput {
            run: run.clone(),
            session: session.clone(),
            system_prompt,
            briefing,
            allowed_tools,
            category,
            complexity,
            control,
            web_search_enabled,
            search_config,
        };

        let outcome = run_agent(self.deps.clone(), input).await;

        state.running.fetch_sub(1, Ordering::SeqCst);
        drop(permit);

        self.finalize(&state, &run, outcome).await;
    }

    /// Search credentials come from the default provider's configuration.
    async fn search_config(&self) -> reverie_core::domain::SearchEngineConfig {
        match self.deps.store.default_provider().await {
            Ok(Some(provider)) => provider.search_config(),
            _ => reverie_core::domain::SearchEngineConfig::default(),
        }
    }

    /// Pause, resume or cancel a session.
    pub async fn control(&self, session_id: &str, action: &str) -> RuntimeResult<String> {
        let state = self
            .session_state(session_id)
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;

        match action {
            "pause" => {
                state.paused.store(true, Ordering::SeqCst);
                for entry in state.agents.iter() {
                    entry.value().control.pause();
                }
                self.deps.store.update_session_status(session_id, SessionStatus::Paused).await?;
                self.deps.sink.emit(SystemEvent::SessionStatusChanged {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Paused.as_str().to_string(),
                });
                Ok("paused".to_string())
            }
            "resume" => {
                state.paused.store(false, Ordering::SeqCst);
                for entry in state.agents.iter() {
                    entry.value().control.resume();
                }
                self.deps.store.update_session_status(session_id, SessionStatus::Running).await?;
                self.deps.sink.emit(SystemEvent::SessionStatusChanged {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Running.as_str().to_string(),
                });
                Ok("resumed".to_string())
            }
            "cancel" => {
                state.cancel.cancel();
                state.paused.store(false, Ordering::SeqCst);
                for entry in state.agents.iter() {
                    entry.value().control.cancel();
                }
                // Release the OS watch as soon as the session is dead.
                if let Some(mut watcher) = state.watcher.lock().take() {
                    watcher.shutdown();
                }
                self.deps.store.update_session_status(session_id, SessionStatus::Failed).await?;
                self.deps.sink.emit(SystemEvent::SessionStatusChanged {
                    session_id: session_id.to_string(),
                    status: SessionStatus::Failed.as_str().to_string(),
                });
                Ok("cancelled".to_string())
            }
            other => Err(RuntimeError::Invalid(format!("unknown control action `{other}`"))),
        }
    }

    /// Live re-route of a single agent.
    pub async fn reroute(
        &self,
        agent_id: &str,
        model_id: Option<String>,
        thinking: Option<ThinkingLevel>,
    ) -> RuntimeResult<()> {
        let handle = self
            .handle(agent_id)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.to_string()))?;

        handle.control.reroute(model_id.clone(), thinking);
        let effective_model = handle.control.model_id();
        let effective_thinking = handle.control.thinking();
        self.deps
            .store
            .update_agent_route(agent_id, &effective_model, effective_thinking)
            .await?;

        if let Ok(Some(run)) = self.deps.store.get_agent(agent_id).await {
            self.deps.sink.emit(SystemEvent::AgentRouteChanged {
                session_id: run.session_id,
                agent_id: agent_id.to_string(),
                model_id: effective_model,
                thinking_level: effective_thinking.as_str().to_string(),
            });
        }
        Ok(())
    }

    /// Build the snapshot the task stage renders.
    pub async fn snapshot(&self, session_id: &str) -> RuntimeResult<SessionSnapshot> {
        let session = self
            .deps
            .store
            .get_session(session_id)
            .await?
            .ok_or_else(|| RuntimeError::SessionNotFound(session_id.to_string()))?;
        let attachments = self.deps.store.list_attachments(session_id).await?;
        let runs = self.deps.store.list_agents(session_id).await?;
        let profiles = self.deps.store.list_model_profiles().await?;

        let agents: Vec<AgentSummary> = runs
            .iter()
            .map(|run| {
                let model_name = profiles
                    .iter()
                    .find(|profile| profile.id == run.model_id)
                    .map(|profile| profile.model_name.clone());
                let activity = self.handle(&run.id).map(|handle| handle.activity());
                AgentSummary::from_run(run, model_name, activity)
            })
            .collect();

        let blackboard = self.deps.store.list_blackboard(session_id, 50).await?;
        let (prompt_tokens, completion_tokens, reasoning_tokens) =
            self.deps.store.session_token_totals(session_id).await?;
        let active = self
            .session_state(session_id)
            .map(|state| state.running.load(Ordering::SeqCst) > 0)
            .unwrap_or(false);

        Ok(SessionSnapshot {
            final_summary: session.final_summary.clone(),
            session,
            attachments,
            agents,
            blackboard,
            totals: TokenTotals { prompt_tokens, completion_tokens, reasoning_tokens },
            active,
        })
    }

    /// Everything the inspector drawer shows for one agent.
    pub async fn agent_detail(&self, agent_id: &str) -> RuntimeResult<AgentDetail> {
        let run = self
            .deps
            .store
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.to_string()))?;
        let profiles = self.deps.store.list_model_profiles().await?;
        let model_name = profiles
            .iter()
            .find(|profile| profile.id == run.model_id)
            .map(|profile| profile.model_name.clone());
        let activity = self.handle(agent_id).map(|handle| handle.activity());

        let messages = reverie_core::llm::ChatMessage::decode_history(&run.message_history_json);
        let attachments = self.deps.store.get_attachments(&run.attached_file_ids).await?;
        let published = self
            .deps
            .store
            .list_blackboard(&run.session_id, 200)
            .await?
            .into_iter()
            .filter(|entry| entry.source_agent_id == agent_id)
            .collect();

        Ok(AgentDetail {
            agent: AgentSummary::from_run(&run, model_name, activity),
            messages,
            attachments,
            published,
        })
    }

    /// A compact text snapshot for the orchestrator's `inspect_swarm` tool.
    pub async fn swarm_status_text(&self, session_id: &str) -> RuntimeResult<String> {
        let runs = self.deps.store.list_agents(session_id).await?;
        if runs.is_empty() {
            return Ok("No agents have been spawned yet.".to_string());
        }
        let mut out = format!("{} agents\n\n", runs.len());
        for run in &runs {
            out.push_str(&format!(
                "- {} [{}] model={} think={} tokens={} status={}\n",
                run.role_name,
                run.archetype.label(),
                if run.model_id.is_empty() { "auto" } else { run.model_id.split(':').next_back().unwrap_or("") },
                run.thinking_level.as_str(),
                run.total_tokens(),
                run.status.as_str()
            ));
            if let Some(handle) = self.handle(&run.id) {
                let activity = handle.activity();
                if !activity.is_empty() {
                    out.push_str(&format!("  doing: {activity}\n"));
                }
            }
            if run.status == AgentStatus::Failed {
                if let Some(error) = &run.error_message {
                    out.push_str(&format!("  error: {}\n", reverie_core::util::truncate_chars(error, 200)));
                }
            }
        }
        Ok(out)
    }

    /// Whether any agent in the session is still working.
    pub async fn has_pending_agents(&self, session_id: &str) -> bool {
        self.has_pending_agents_except(session_id, "").await
    }

    /// Whether any agent other than `exclude` is still working.
    ///
    /// `finalize_task` runs *inside* an agent, so that agent must not count as
    /// its own straggler or the wait would always time out.
    pub async fn has_pending_agents_except(&self, session_id: &str, exclude: &str) -> bool {
        match self.deps.store.list_agents(session_id).await {
            Ok(runs) => runs.iter().any(|run| {
                run.id != exclude
                    && matches!(
                        run.status,
                        AgentStatus::Queued | AgentStatus::Running | AgentStatus::WaitingDependency
                    )
            }),
            Err(_) => false,
        }
    }

    /// Record the session's final deliverable.
    pub async fn set_final_summary(&self, session_id: &str, summary: &str) -> RuntimeResult<()> {
        self.deps.store.update_session_summary(session_id, summary).await?;
        if let Some(state) = self.session_state(session_id) {
            *state.final_summary.write() = Some(summary.to_string());
            state.finished.store(true, Ordering::SeqCst);
        }
        self.deps.store.update_session_status(session_id, SessionStatus::Completed).await?;
        self.deps.sink.emit(SystemEvent::SessionCompleted {
            session_id: session_id.to_string(),
            summary: summary.to_string(),
        });
        self.deps.sink.emit(SystemEvent::SessionStatusChanged {
            session_id: session_id.to_string(),
            status: SessionStatus::Completed.as_str().to_string(),
        });
        Ok(())
    }

    /// Publish an agent's deliverable onto the blackboard.
    pub async fn publish_finding(
        &self,
        session_id: &str,
        agent_id: &str,
        tag: &str,
        summary: &str,
        content: &str,
    ) -> RuntimeResult<()> {
        let entry = BlackboardEntry::publish(
            session_id,
            agent_id,
            tag.to_string(),
            summary.to_string(),
            content.to_string(),
            vec!["*".to_string()],
        );
        self.deps.store.insert_blackboard(&entry).await?;
        self.deps.sink.emit(SystemEvent::BlackboardPublished {
            session_id: session_id.to_string(),
            entry_id: entry.id,
            topic_tag: tag.to_string(),
            source_agent_id: agent_id.to_string(),
            summary: summary.to_string(),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complexity_follows_archetype() {
        let orchestrator = AgentSpec::new("o", AgentArchetype::MetaOrchestrator, "plan");
        let extractor = AgentSpec::new("e", AgentArchetype::DataExtractor, "scrape");
        assert!(estimate_complexity(&orchestrator) > estimate_complexity(&extractor));
        assert!(estimate_complexity(&extractor) >= 0.05);
    }

    #[test]
    fn long_tasks_are_slightly_harder() {
        let short = AgentSpec::new("a", AgentArchetype::Coder, "fix");
        let long = AgentSpec::new("b", AgentArchetype::Coder, "x".repeat(4000));
        assert!(estimate_complexity(&long) > estimate_complexity(&short));
    }

    #[test]
    fn config_tracks_session_flags() {
        let mut session = Session::create("t", "F:/ws", "o", "high", false);
        session.isolation_strategy = IsolationStrategy::SandboxDir;
        let config = SessionRunConfig::from_session(&session);
        assert!(!config.web_search_enabled);
        assert_eq!(config.thinking_override, Some(ThinkingLevel::High));
        assert_eq!(config.isolation, IsolationStrategy::SandboxDir);
    }

    #[test]
    fn handle_activity_is_mutable() {
        let spec = AgentSpec::new("r", AgentArchetype::Researcher, "t");
        let run = AgentRun::from_spec("s", None, &spec, "F:/ws");
        let handle = AgentHandle::new(&run);
        assert_eq!(handle.activity(), "Queued");
        handle.set_activity("Searching");
        assert_eq!(handle.activity(), "Searching");
    }
}
