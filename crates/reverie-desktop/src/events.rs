//! Bridges the core event stream onto Tauri's IPC channel.

use reverie_core::events::{EventSink, SystemEvent, EVENT_CHANNEL};
use tauri::{AppHandle, Emitter};

/// Emits every [`SystemEvent`] as a `reverie-event` message.
#[derive(Clone)]
pub struct TauriEventSink {
    app: AppHandle,
}

impl TauriEventSink {
    pub fn new(app: AppHandle) -> Self {
        TauriEventSink { app }
    }
}

impl EventSink for TauriEventSink {
    fn emit(&self, event: SystemEvent) {
        if let Err(err) = self.app.emit(EVENT_CHANNEL, &event) {
            tracing::debug!(kind = event.kind(), %err, "failed to emit system event");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_name_is_stable() {
        assert_eq!(EVENT_CHANNEL, "reverie-event");
    }
}
