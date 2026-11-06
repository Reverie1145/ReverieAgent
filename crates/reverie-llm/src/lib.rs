//! Multi-protocol LLM gateway, thinking-depth adaptation, model profiling and
//! dynamic routing for ReverieAgent.

pub mod adapters;
pub mod error;
pub mod gateway;
pub mod profiler;
pub mod router;
pub mod stream_util;

pub use error::{LlmError, LlmResult};
pub use gateway::{Completion, LlmGateway};
pub use profiler::ModelProfilerService;
pub use router::{ModelRouter, RouteDecision, RouteRequest};
