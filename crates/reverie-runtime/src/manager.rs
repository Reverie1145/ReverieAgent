//! Application state: wires the store, gateway, tool registry, scheduler and
//! evolution engine into one shareable object.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use reverie_core::domain::{Session, SessionAttachment, SessionStatus};
use reverie_core::events::{EventSink, SystemEvent};
use reverie_core::{DB_FILE_NAME, DEFAULT_MAX_CONCURRENT_AGENTS};
use reverie_llm::{LlmGateway, ModelProfilerService};
use reverie_store::Store;
use reverie_tools::{build_registry_with_dynamic, GlobalToolRegistry, OrchestrationBridge, VfsLockManager};

use crate::agent_actor::ActorDeps;
use crate::bridge::SchedulerBridge;
use crate::evolution::EvolutionEngine;
use crate::scheduler::{RuntimeError, RuntimeResult, SwarmScheduler};

/// Request payload for creating a task.
#[derive(Debug, Clone)]
pub struct CreateSessionRequest {
    pub title: String,
    pub workspace_path: String,
    pub user_objective: String,
    pub attachment_ids: Vec<String>,
    pub thinking_mode_override: String,
    pub web_search_enabled: bool,
    /// Optional explicit model for the Meta-Orchestrator. `None` lets the
    /// router choose.
    pub orchestrator_model_id: Option<String>,
}

impl CreateSessionRequest {
    /// Derive a title from the objective when the user did not supply one.
    pub fn effective_title(&self) -> String {
        let title = self.title.trim();
        if !title.is_empty() {
            return title.to_string();
        }
        reverie_core::util::truncate_chars(self.user_objective.trim(), 60)
    }
}

/// Shared application state.
pub struct AppState {
    pub store: Arc<Store>,
    pub gateway: Arc<LlmGateway>,
    pub registry: Arc<GlobalToolRegistry>,
    pub profiler: Arc<ModelProfilerService>,
    pub scheduler: Arc<SwarmScheduler>,
    pub evolution: Arc<EvolutionEngine>,
    pub bridge: Arc<SchedulerBridge>,
    pub sink: Arc<dyn EventSink>,
    pub http: reqwest::Client,
    pub vfs: Arc<VfsLockManager>,
    pub db_path: PathBuf,
    /// Attachments staged before a session exists. They are only written to the
    /// database once bound to a real session, which keeps the foreign key
    /// invariant intact.
    pub staged: DashMap<String, SessionAttachment>,
}

impl AppState {
    /// Open the database and build every subsystem.
    pub async fn bootstrap(db_path: impl AsRef<Path>, sink: Arc<dyn EventSink>) -> RuntimeResult<Arc<Self>> {
        let db_path = db_path.as_ref().to_path_buf();
        let store = Arc::new(Store::connect(&db_path).await?);

        let http = reverie_llm::gateway::build_client()
            .map_err(|err| RuntimeError::Other(format!("cannot build HTTP client: {err}")))?;
        let gateway = Arc::new(LlmGateway::with_client(http.clone()));
        let registry = build_registry_with_dynamic(&store).await;
        let vfs = Arc::new(VfsLockManager::new());

        let bridge = SchedulerBridge::new();
        let orchestration: Arc<dyn OrchestrationBridge> = bridge.clone();

        let deps = Arc::new(ActorDeps {
            store: store.clone(),
            gateway: gateway.clone(),
            registry: registry.clone(),
            sink: sink.clone(),
            vfs: vfs.clone(),
            http: http.clone(),
            orchestration,
        });

        let scheduler = SwarmScheduler::new(deps, DEFAULT_MAX_CONCURRENT_AGENTS);
        bridge.attach(&scheduler);

        let profiler = Arc::new(ModelProfilerService::new(gateway.clone(), store.clone()));
        let evolution = Arc::new(EvolutionEngine::new(store.clone(), gateway.clone()));

        if let Ok(created) = evolution.seed_prompts().await {
            if created > 0 {
                sink.emit(SystemEvent::LogMessage {
                    level: "info".to_string(),
                    scope: "bootstrap".to_string(),
                    message: format!("seeded {created} archetype prompts"),
                });
            }
        }

        Ok(Arc::new(AppState {
            store,
            gateway,
            registry,
            profiler,
            scheduler,
            evolution,
            bridge,
            sink,
            http,
            vfs,
            db_path,
            staged: DashMap::new(),
        }))
    }

    /// Remember attachments staged before a session exists.
    pub fn stage_attachments(&self, attachments: Vec<SessionAttachment>) {
        for attachment in attachments {
            self.staged.insert(attachment.id.clone(), attachment);
        }
    }

    /// Remove staged attachments from the registry so they can be bound.
    pub fn take_staged(&self, ids: &[String]) -> Vec<SessionAttachment> {
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some((_, value)) = self.staged.remove(id) {
                out.push(value);
            }
        }
        out
    }

    /// Number of attachments currently waiting to be bound.
    pub fn staged_count(&self) -> usize {
        self.staged.len()
    }

    /// Default database path inside an application data directory.
    pub fn default_db_path(app_data_dir: impl AsRef<Path>) -> PathBuf {
        app_data_dir.as_ref().join(DB_FILE_NAME)
    }

    /// Create a session, bind its attachments and start the swarm.
    pub async fn create_and_start_session(&self, request: CreateSessionRequest) -> RuntimeResult<Session> {
        if request.user_objective.trim().is_empty() {
            return Err(RuntimeError::Invalid("the task objective must not be empty".to_string()));
        }
        let workspace = PathBuf::from(&request.workspace_path);
        if request.workspace_path.trim().is_empty() {
            return Err(RuntimeError::Invalid("a workspace folder is required".to_string()));
        }
        std::fs::create_dir_all(&workspace)
            .map_err(|err| RuntimeError::Other(format!("cannot create workspace: {err}")))?;
        std::fs::create_dir_all(workspace.join(reverie_core::RUNTIME_DIR))
            .map_err(|err| RuntimeError::Other(format!("cannot create .reverie dir: {err}")))?;

        let override_value = Session::normalize_override(&request.thinking_mode_override)
            .map_err(|err| RuntimeError::Invalid(err.to_string()))?;

        let mut session = Session::create(
            request.effective_title(),
            workspace.to_string_lossy().to_string(),
            request.user_objective.trim(),
            override_value,
            request.web_search_enabled,
        );
        session.status = SessionStatus::Planning;

        self.store.insert_session(&session).await?;

        let staged = self.take_staged(&request.attachment_ids);
        if !staged.is_empty() {
            let bound: Vec<SessionAttachment> = staged
                .into_iter()
                .map(|mut attachment| {
                    attachment.session_id = session.id.clone();
                    attachment
                })
                .collect();
            self.store.insert_attachments(&bound).await?;
        } else if !request.attachment_ids.is_empty() {
            // Attachments may already be persisted (e.g. a retried submit).
            self.store.bind_attachments(&request.attachment_ids, &session.id).await?;
        }

        self.scheduler
            .start_session(&session, request.orchestrator_model_id.clone())
            .await?;
        Ok(session)
    }

    /// Reflect on a finished session and fold the lessons back in.
    pub async fn reflect(&self, session_id: &str) -> RuntimeResult<()> {
        let sink = self.sink.clone();
        self.evolution
            .reflect_session(session_id, sink.as_ref())
            .await
            .map(|_| ())
            .map_err(RuntimeError::Store)
    }

    /// Flush the WAL on shutdown.
    pub async fn shutdown(&self) {
        let _ = self.store.checkpoint().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::events::NoopSink;

    fn temp_db() -> PathBuf {
        std::env::temp_dir()
            .join(format!("reverie-app-{}", uuid::Uuid::new_v4()))
            .join("reverie_agent.db")
    }

    #[tokio::test]
    async fn bootstrap_creates_every_subsystem() {
        let state = AppState::bootstrap(temp_db(), Arc::new(NoopSink)).await.unwrap();
        assert_eq!(state.registry.len(), 16);
        assert_eq!(state.store.count_evolution_records().await.unwrap(), 12);
        let _ = std::fs::remove_dir_all(state.db_path.parent().unwrap());
    }

    #[tokio::test]
    async fn session_requires_an_objective() {
        let state = AppState::bootstrap(temp_db(), Arc::new(NoopSink)).await.unwrap();
        let request = CreateSessionRequest {
            title: "t".into(),
            workspace_path: std::env::temp_dir().to_string_lossy().to_string(),
            user_objective: "  ".into(),
            attachment_ids: vec![],
            thinking_mode_override: "auto".into(),
            web_search_enabled: true,
            orchestrator_model_id: None,
        };
        assert!(state.create_and_start_session(request).await.is_err());
        let _ = std::fs::remove_dir_all(state.db_path.parent().unwrap());
    }

    #[tokio::test]
    async fn session_creation_persists_and_binds_attachments() {
        let state = AppState::bootstrap(temp_db(), Arc::new(NoopSink)).await.unwrap();
        let workspace = std::env::temp_dir().join(format!("reverie-ws-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();

        let attachment = reverie_core::domain::SessionAttachment::staged("a.txt", "/tmp/a.txt", "text/plain", 5);
        state.stage_attachments(vec![attachment.clone()]);
        assert_eq!(state.staged_count(), 1);

        let request = CreateSessionRequest {
            title: "".into(),
            workspace_path: workspace.to_string_lossy().to_string(),
            user_objective: "do something useful".into(),
            attachment_ids: vec![attachment.id.clone()],
            thinking_mode_override: "high".into(),
            web_search_enabled: false,
            orchestrator_model_id: None,
        };
        let session = state.create_and_start_session(request).await.unwrap();
        assert_eq!(session.title, "do something useful");
        assert_eq!(session.thinking_mode_override, "high");
        assert_eq!(state.store.count_attachments(&session.id).await.unwrap(), 1);
        assert_eq!(state.staged_count(), 0);
        assert!(workspace.join(".reverie").exists());

        let _ = std::fs::remove_dir_all(&workspace);
        let _ = std::fs::remove_dir_all(state.db_path.parent().unwrap());
    }

    #[test]
    fn title_falls_back_to_objective() {
        let request = CreateSessionRequest {
            title: "   ".into(),
            workspace_path: "F:/ws".into(),
            user_objective: "Build a distributed crawler with retries".into(),
            attachment_ids: vec![],
            thinking_mode_override: "auto".into(),
            web_search_enabled: true,
            orchestrator_model_id: None,
        };
        assert!(request.effective_title().starts_with("Build a distributed"));
    }
}
