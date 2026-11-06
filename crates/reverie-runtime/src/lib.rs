//! The orchestration runtime: actor scheduler, meta-agent triad, workspace
//! warden and the self-evolution engine.

pub mod agent_actor;
pub mod bridge;
pub mod context_broker;
pub mod control;
pub mod evolution;
pub mod finalize;
pub mod manager;
pub mod prompt;
pub mod scheduler;
pub mod snapshot;
pub mod stager;
pub mod warden;
pub mod watcher;

pub use agent_actor::{ActorDeps, ActorInput, AgentOutcome};
pub use bridge::SchedulerBridge;
pub use context_broker::{BrokerRequest, ContextBrokerAgent};
pub use control::AgentControl;
pub use evolution::{EvolutionEngine, EvolutionReport};
pub use manager::{AppState, CreateSessionRequest};
pub use prompt::{build_system_prompt, default_prompt};
pub use scheduler::{RuntimeError, RuntimeResult, SessionState, SwarmScheduler};
pub use snapshot::{AgentDetail, AgentSummary, SessionSnapshot, TokenTotals};
pub use stager::AttachmentStager;
pub use warden::{EnvironmentWardenAgent, MergeOutcome, ProvisionedWorkspace};
