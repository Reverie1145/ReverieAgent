//! Per-agent runtime controls that can be flipped while the agent is running.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use reverie_core::llm::ThinkingLevel;
use tokio_util::sync::CancellationToken;

/// Live, mutable knobs for one agent.
#[derive(Debug, Clone)]
pub struct AgentControl {
    cancel: CancellationToken,
    paused: Arc<AtomicBool>,
    model_id: Arc<RwLock<String>>,
    thinking: Arc<RwLock<ThinkingLevel>>,
}

impl AgentControl {
    pub fn new(model_id: impl Into<String>, thinking: ThinkingLevel) -> Self {
        AgentControl {
            cancel: CancellationToken::new(),
            paused: Arc::new(AtomicBool::new(false)),
            model_id: Arc::new(RwLock::new(model_id.into())),
            thinking: Arc::new(RwLock::new(thinking)),
        }
    }

    pub fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub fn model_id(&self) -> String {
        self.model_id.read().clone()
    }

    pub fn set_model_id(&self, model_id: impl Into<String>) {
        *self.model_id.write() = model_id.into();
    }

    pub fn thinking(&self) -> ThinkingLevel {
        *self.thinking.read()
    }

    pub fn set_thinking(&self, level: ThinkingLevel) {
        *self.thinking.write() = level;
    }

    /// Switch both knobs at once (used by the inspector's live re-route).
    pub fn reroute(&self, model_id: Option<String>, thinking: Option<ThinkingLevel>) {
        if let Some(model_id) = model_id {
            self.set_model_id(model_id);
        }
        if let Some(thinking) = thinking {
            self.set_thinking(thinking);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knobs_are_independently_mutable() {
        let control = AgentControl::new("p:m", ThinkingLevel::Low);
        control.set_thinking(ThinkingLevel::Max);
        control.set_model_id("p:other");
        assert_eq!(control.thinking(), ThinkingLevel::Max);
        assert_eq!(control.model_id(), "p:other");
    }

    #[test]
    fn pause_and_cancel_are_observable() {
        let control = AgentControl::new("m", ThinkingLevel::Off);
        assert!(!control.is_paused());
        control.pause();
        assert!(control.is_paused());
        control.resume();
        assert!(!control.is_paused());
        assert!(!control.is_cancelled());
        control.cancel();
        assert!(control.is_cancelled());
    }

    #[test]
    fn reroute_applies_only_provided_values() {
        let control = AgentControl::new("m1", ThinkingLevel::Low);
        control.reroute(None, Some(ThinkingLevel::High));
        assert_eq!(control.model_id(), "m1");
        assert_eq!(control.thinking(), ThinkingLevel::High);
    }

    #[test]
    fn clones_share_state() {
        let control = AgentControl::new("m", ThinkingLevel::Low);
        let clone = control.clone();
        clone.set_thinking(ThinkingLevel::Max);
        assert_eq!(control.thinking(), ThinkingLevel::Max);
    }
}
