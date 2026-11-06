//! IPC command surface exposed to the React frontend.

pub mod agents;
pub mod app;
pub mod attachments;
pub mod evolution;
pub mod models;
pub mod providers;
pub mod sessions;
pub mod tools;

use std::sync::Arc;

use reverie_runtime::AppState;

/// Shared handle managed by Tauri.
pub type Shared = Arc<AppState>;
