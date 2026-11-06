//! The dual-tier tooling engine.
//!
//! Tier 1 is twelve native Rust primitives; tier 2 is the runtime tool forge.
//! Both are exposed through one [`GlobalToolRegistry`] so a sub-agent cannot
//! tell them apart.

pub mod args;
pub mod attachment_tool;
pub mod blackboard_tools;
pub mod builtin;
pub mod context;
pub mod diff;
pub mod error;
pub mod forge;
pub mod fs_tools;
pub mod http;
pub mod meta_tools;
pub mod registry;
pub mod shell;
pub mod vfs;
pub mod web;

pub use builtin::{build_registry, build_registry_with_dynamic, META_TOOL_NAMES, NATIVE_TOOL_NAMES};
pub use context::{OrchestrationBridge, ToolExecutionContext, DEFAULT_MAX_OUTPUT_CHARS};
pub use diff::{render_bundle, totals, unified_diff, FileDiff};
pub use error::{ToolError, ToolResult};
pub use registry::{AgentTool, GlobalToolRegistry, ToolCategory};
pub use vfs::VfsLockManager;
