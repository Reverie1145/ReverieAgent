//! Domain primitives shared across every ReverieAgent crate.
//!
//! This crate is intentionally free of I/O: it holds the canonical value
//! objects, enums and error taxonomy that the store, gateway, tool runtime and
//! desktop bridge all agree on.

pub mod domain;
pub mod error;
pub mod events;
pub mod llm;
pub mod time;
pub mod util;

pub use error::{CoreError, CoreResult};

/// Canonical database file name for the embedded SQLite store.
pub const DB_FILE_NAME: &str = "reverie_agent.db";

/// Directory (relative to a session workspace) that holds runtime state.
pub const RUNTIME_DIR: &str = ".reverie";

/// Maximum number of concurrent sub-agents the scheduler will keep in flight
/// before it starts queueing. The scheduler itself is unbounded by design; this
/// is the default soft ceiling used by the runtime supervisor.
pub const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 512;
