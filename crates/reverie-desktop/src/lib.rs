//! Tauri v2 shell for ReverieAgent.

pub mod commands;
pub mod dto;
pub mod events;

use std::sync::{Arc, OnceLock};

use reverie_core::events::{EventSink, SystemEvent};
use reverie_core::DB_FILE_NAME;
use reverie_runtime::AppState;
use tauri::Manager;

use crate::events::TauriEventSink;

/// Set once the Tauri app is ready, so the panic hook can tell the UI.
static PANIC_SINK: OnceLock<Arc<dyn EventSink>> = OnceLock::new();

/// A panic inside a command aborts the task without resolving the JS promise,
/// which would leave the window spinning forever. Surface it instead.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|loc| format!("{}:{}", loc.file(), loc.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let payload = if let Some(text) = info.payload().downcast_ref::<&str>() {
            (*text).to_string()
        } else if let Some(text) = info.payload().downcast_ref::<String>() {
            text.clone()
        } else {
            "unknown panic".to_string()
        };
        let message = format!("{payload} (at {location})");
        eprintln!("[reverie panic] {message}");

        if let Some(sink) = PANIC_SINK.get() {
            sink.emit(SystemEvent::LogMessage {
                level: "error".to_string(),
                scope: "backend".to_string(),
                message: reverie_core::util::truncate_chars(&message, 300),
            });
        }
    }));
}

/// Build and run the desktop application.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "reverie=info,warn".into()),
        )
        .init();

    install_panic_hook();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let sink: Arc<dyn EventSink> = Arc::new(TauriEventSink::new(handle.clone()));
            let _ = PANIC_SINK.set(sink.clone());

            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("ReverieAgent"));
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join(DB_FILE_NAME);

            let state = tauri::async_runtime::block_on(AppState::bootstrap(db_path, sink))?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::providers::cmd_list_providers,
            commands::providers::cmd_upsert_provider,
            commands::providers::cmd_delete_provider,
            commands::providers::cmd_update_provider_search_config,
            commands::models::cmd_pull_provider_models,
            commands::models::cmd_profile_provider_models,
            commands::models::cmd_update_model_profile_settings,
            commands::models::cmd_list_model_profiles,
            commands::sessions::cmd_create_and_start_session,
            commands::sessions::cmd_list_sessions,
            commands::sessions::cmd_get_session_snapshot,
            commands::sessions::cmd_control_session,
            commands::sessions::cmd_delete_session,
            commands::sessions::cmd_reflect_session,
            commands::agents::cmd_update_agent_thinking_or_model,
            commands::agents::cmd_get_agent_detail,
            commands::attachments::cmd_stage_attachments,
            commands::attachments::cmd_stage_attachment_bytes,
            commands::tools::cmd_list_all_tools,
            commands::evolution::cmd_list_evolution_records,
            commands::evolution::cmd_save_evolution_record,
            commands::app::cmd_get_app_info,
        ])
        .run(tauri::generate_context!())
        .expect("failed to launch ReverieAgent");
}
