//! Canonical domain entities. These mirror the SQLite schema one-to-one and are
//! the only types that cross crate boundaries.

pub mod agent;
pub mod attachment;
pub mod blackboard;
pub mod evolution;
pub mod model;
pub mod provider;
pub mod session;
pub mod tool;

pub use agent::{AgentArchetype, AgentRun, AgentSpec, AgentStatus};
pub use attachment::{AttachmentKind, SessionAttachment};
pub use blackboard::BlackboardEntry;
pub use evolution::{EvolutionRecord, EvolutionRecordType};
pub use model::{CostTier, ModelProfile, TaskCategory};
pub use provider::{Provider, Protocol, SearchEngineConfig};
pub use session::{ContextStrategy, IsolationStrategy, Session, SessionStatus, TopologyMode};
pub use tool::{DynamicToolRecord, Interpreter, ToolDescriptor, ToolOutcome};
