//! Bridges the tool runtime's orchestration tools to the swarm scheduler.
//!
//! The dependency runs one way (runtime → tools), so the bridge is attached
//! after construction via a `Weak` handle to break the cycle.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::RwLock;
use reverie_core::domain::{AgentSpec, ContextStrategy, IsolationStrategy, TopologyMode};
use reverie_core::events::SystemEvent;
use reverie_tools::{OrchestrationBridge, ToolError, ToolResult};

use crate::scheduler::SwarmScheduler;

/// How long `finalize_task` waits for stragglers before giving up.
const FINALIZE_WAIT: Duration = Duration::from_secs(240);
const FINALIZE_POLL: Duration = Duration::from_secs(1);

/// Late-bound handle to the scheduler.
#[derive(Debug)]
pub struct SchedulerBridge {
    scheduler: RwLock<Weak<SwarmScheduler>>,
}

impl Default for SchedulerBridge {
    fn default() -> Self {
        SchedulerBridge { scheduler: RwLock::new(Weak::new()) }
    }
}

impl SchedulerBridge {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Connect the bridge once the scheduler exists.
    pub fn attach(&self, scheduler: &Arc<SwarmScheduler>) {
        *self.scheduler.write() = Arc::downgrade(scheduler);
    }

    fn get(&self) -> ToolResult<Arc<SwarmScheduler>> {
        self.scheduler
            .read()
            .upgrade()
            .ok_or_else(|| ToolError::unavailable("the swarm scheduler is not running"))
    }
}

fn runtime_err(err: crate::scheduler::RuntimeError) -> ToolError {
    ToolError::other(err.to_string())
}

#[async_trait]
impl OrchestrationBridge for SchedulerBridge {
    async fn configure_strategy(
        &self,
        session_id: &str,
        topology: &str,
        context_strategy: &str,
        isolation: &str,
    ) -> ToolResult<String> {
        let topology = TopologyMode::parse(topology).map_err(|err| ToolError::invalid(err.to_string()))?;
        let context =
            ContextStrategy::parse(context_strategy).map_err(|err| ToolError::invalid(err.to_string()))?;
        let isolation_mode =
            IsolationStrategy::parse(isolation).map_err(|err| ToolError::invalid(err.to_string()))?;
        let scheduler = self.get()?;

        scheduler
            .deps
            .store
            .update_session_strategy(session_id, topology, context, isolation_mode)
            .await
            .map_err(|err| ToolError::Store(err.to_string()))?;

        if let Some(state) = scheduler.session_state(session_id) {
            let mut config = state.config.write();
            config.topology = topology;
            config.context_strategy = context;
            config.isolation = isolation_mode;
        }

        scheduler.deps.sink.emit(SystemEvent::SessionStrategyChanged {
            session_id: session_id.to_string(),
            topology: topology.as_str().to_string(),
            context_strategy: context.as_str().to_string(),
            isolation: isolation_mode.as_str().to_string(),
        });

        Ok(format!(
            "Strategy set to {} / {} / {}. New agents will be spawned under this wiring.",
            topology.label(),
            context.label(),
            isolation_mode.label()
        ))
    }

    async fn spawn_sub_agents(
        &self,
        session_id: &str,
        parent_agent_id: &str,
        specs: Vec<AgentSpec>,
    ) -> ToolResult<Vec<String>> {
        let scheduler = self.get()?;
        scheduler
            .spawn_batch(session_id, parent_agent_id, specs)
            .await
            .map_err(runtime_err)
    }

    async fn finalize_session(&self, session_id: &str, agent_id: &str, summary: &str) -> ToolResult<()> {
        let scheduler = self.get()?;

        // Give outstanding sub-agents a chance to land before closing the books.
        let deadline = Instant::now() + FINALIZE_WAIT;
        while Instant::now() < deadline {
            if !scheduler.has_pending_agents_except(session_id, agent_id).await {
                break;
            }
            tokio::time::sleep(FINALIZE_POLL).await;
        }

        if scheduler.has_pending_agents_except(session_id, agent_id).await {
            scheduler.deps.sink.emit(SystemEvent::LogMessage {
                level: "warn".to_string(),
                scope: "Meta-Orchestrator".to_string(),
                message: "finalising while some sub-agents are still running".to_string(),
            });
        }

        scheduler
            .publish_finding(session_id, agent_id, "final-report", "Final deliverable", summary)
            .await
            .map_err(runtime_err)?;
        scheduler.set_final_summary(session_id, summary).await.map_err(runtime_err)
    }

    async fn swarm_status(&self, session_id: &str) -> ToolResult<String> {
        let scheduler = self.get()?;
        scheduler.swarm_status_text(session_id).await.map_err(runtime_err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unattached_bridge_reports_unavailable() {
        let bridge = SchedulerBridge::new();
        let err = bridge.swarm_status("s").await.err().expect("must fail");
        assert!(err.to_string().contains("not running"));
    }

    #[tokio::test]
    async fn invalid_enum_is_rejected_before_scheduler_access() {
        let bridge = SchedulerBridge::new();
        let err = bridge
            .configure_strategy("s", "hypercube", "blackboard", "shared_vfs_lock")
            .await
            .err()
            .expect("must fail");
        assert!(err.to_string().contains("invalid arguments"));
    }
}
