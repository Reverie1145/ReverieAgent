//! Model capability matrix: what a model can do, how fast it is, and what it
//! costs. Populated by the bootstrap profiler and refined by the evolution
//! engine from empirical telemetry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::llm::ThinkingLevel;
use crate::time;

/// Coarse task taxonomy used by the router to pick a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    Orchestration,
    Architecture,
    Coding,
    Debugging,
    Research,
    Extraction,
    VisionInspection,
    Summarization,
    General,
}

impl TaskCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskCategory::Orchestration => "orchestration",
            TaskCategory::Architecture => "architecture",
            TaskCategory::Coding => "coding",
            TaskCategory::Debugging => "debugging",
            TaskCategory::Research => "research",
            TaskCategory::Extraction => "extraction",
            TaskCategory::VisionInspection => "vision_inspection",
            TaskCategory::Summarization => "summarization",
            TaskCategory::General => "general",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "orchestration" | "planning" => Ok(TaskCategory::Orchestration),
            "architecture" | "design" => Ok(TaskCategory::Architecture),
            "coding" | "implementation" => Ok(TaskCategory::Coding),
            "debugging" | "diagnosis" => Ok(TaskCategory::Debugging),
            "research" | "search" => Ok(TaskCategory::Research),
            "extraction" | "scraping" | "cleaning" => Ok(TaskCategory::Extraction),
            "vision" | "vision_inspection" | "image" => Ok(TaskCategory::VisionInspection),
            "summarization" | "summary" => Ok(TaskCategory::Summarization),
            "general" | "chat" => Ok(TaskCategory::General),
            other => Err(CoreError::InvalidEnum { kind: "TaskCategory", value: other.to_string() }),
        }
    }

    /// The score column that best predicts quality for this category.
    pub fn primary_score(self) -> ScoreAxis {
        match self {
            TaskCategory::Orchestration | TaskCategory::Architecture => ScoreAxis::Reasoning,
            TaskCategory::Coding | TaskCategory::Debugging => ScoreAxis::Coding,
            TaskCategory::Research | TaskCategory::Extraction => ScoreAxis::Extraction,
            TaskCategory::VisionInspection => ScoreAxis::Coding,
            TaskCategory::Summarization | TaskCategory::General => ScoreAxis::Reasoning,
        }
    }
}

/// Which capability score a task category cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoreAxis {
    Reasoning,
    Coding,
    Extraction,
}

/// 1 = cheap high-concurrency worker, 2 = balanced, 3 = frontier flagship.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CostTier(pub i64);

impl Default for CostTier {
    fn default() -> Self {
        CostTier(2)
    }
}

impl CostTier {
    pub fn new(value: i64) -> Self {
        CostTier(value.clamp(1, 3))
    }

    pub fn label(self) -> &'static str {
        match self.0 {
            1 => "Low cost",
            3 => "Frontier",
            _ => "Balanced",
        }
    }
}

/// A profiled model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub id: String,
    pub provider_id: String,
    pub model_name: String,
    pub context_window: i64,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_thinking: bool,
    pub supported_thinking_levels: Vec<ThinkingLevel>,
    pub default_thinking_level: ThinkingLevel,
    pub latency_ms: i64,
    pub reasoning_score: f64,
    pub coding_score: f64,
    pub extraction_score: f64,
    pub cost_tier: CostTier,
    pub total_calls: i64,
    pub success_calls: i64,
    pub task_fit_notes: String,
    pub updated_at: DateTime<Utc>,
}

impl ModelProfile {
    /// Deterministic primary key: `{provider_id}:{model_name}`.
    pub fn compose_id(provider_id: &str, model_name: &str) -> String {
        format!("{provider_id}:{model_name}")
    }

    /// A freshly discovered model, before probing.
    pub fn provisional(provider_id: &str, model_name: &str) -> Self {
        ModelProfile {
            id: ModelProfile::compose_id(provider_id, model_name),
            provider_id: provider_id.to_string(),
            model_name: model_name.to_string(),
            context_window: 32768,
            supports_tools: true,
            supports_vision: false,
            supports_thinking: false,
            supported_thinking_levels: vec![ThinkingLevel::Off],
            default_thinking_level: ThinkingLevel::Off,
            latency_ms: 1000,
            reasoning_score: 5.0,
            coding_score: 5.0,
            extraction_score: 5.0,
            cost_tier: CostTier::default(),
            total_calls: 0,
            success_calls: 0,
            task_fit_notes: "Discovered but not yet profiled. Run the auto-profiler to populate capabilities."
                .to_string(),
            updated_at: time::now(),
        }
    }

    /// Score for a given axis.
    pub fn score_for(&self, axis: ScoreAxis) -> f64 {
        match axis {
            ScoreAxis::Reasoning => self.reasoning_score,
            ScoreAxis::Coding => self.coding_score,
            ScoreAxis::Extraction => self.extraction_score,
        }
    }

    /// Empirical success rate in `[0, 1]`; neutral 0.75 before any samples.
    pub fn success_rate(&self) -> f64 {
        if self.total_calls <= 0 {
            0.75
        } else {
            (self.success_calls as f64 / self.total_calls as f64).clamp(0.0, 1.0)
        }
    }

    /// Context window in tokens, floored at a sane minimum.
    pub fn effective_context_window(&self) -> i64 {
        self.context_window.max(2048)
    }

    /// Clamp a requested thinking level to what the model actually supports.
    pub fn resolve_thinking(&self, requested: ThinkingLevel) -> ThinkingLevel {
        if !self.supports_thinking {
            return ThinkingLevel::Off;
        }
        requested.clamp_to(&self.supported_thinking_levels)
    }

    /// Whether the model can accept image content parts.
    pub fn can_see(self_or_ref: &Self) -> bool {
        self_or_ref.supports_vision
    }

    /// Recompute the rolling success statistics after a completed run.
    pub fn record_outcome(&mut self, success: bool) {
        self.total_calls += 1;
        if success {
            self.success_calls += 1;
        }
        self.updated_at = time::now();
    }

    /// Blend a newly measured latency into the stored value (exponential
    /// moving average, α = 0.3) so transient spikes do not dominate.
    pub fn observe_latency(&mut self, measured_ms: i64) {
        if measured_ms <= 0 {
            return;
        }
        let blended = (self.latency_ms as f64) * 0.7 + (measured_ms as f64) * 0.3;
        self.latency_ms = blended.round().max(1.0) as i64;
    }

    /// A compact human label, e.g. `deepseek-chat · Think: Off`.
    pub fn display_label(&self) -> String {
        format!("{} · Think: {}", self.model_name, capitalize(self.default_thinking_level.as_str()))
    }
}

pub fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_composite() {
        assert_eq!(ModelProfile::compose_id("p1", "gpt-4o"), "p1:gpt-4o");
    }

    #[test]
    fn thinking_is_clamped_for_non_reasoning_models() {
        let profile = ModelProfile::provisional("p", "m");
        assert_eq!(profile.resolve_thinking(ThinkingLevel::Max), ThinkingLevel::Off);
    }

    #[test]
    fn thinking_clamps_to_supported_subset() {
        let mut profile = ModelProfile::provisional("p", "m");
        profile.supports_thinking = true;
        profile.supported_thinking_levels =
            vec![ThinkingLevel::Off, ThinkingLevel::Low, ThinkingLevel::Medium];
        assert_eq!(profile.resolve_thinking(ThinkingLevel::Max), ThinkingLevel::Medium);
        assert_eq!(profile.resolve_thinking(ThinkingLevel::Low), ThinkingLevel::Low);
    }

    #[test]
    fn latency_uses_ema() {
        let mut profile = ModelProfile::provisional("p", "m");
        profile.latency_ms = 1000;
        profile.observe_latency(2000);
        assert_eq!(profile.latency_ms, 1300);
        profile.observe_latency(0);
        assert_eq!(profile.latency_ms, 1300);
    }

    #[test]
    fn outcome_tracking_computes_rate() {
        let mut profile = ModelProfile::provisional("p", "m");
        profile.record_outcome(true);
        profile.record_outcome(false);
        profile.record_outcome(true);
        assert_eq!(profile.total_calls, 3);
        assert!((profile.success_rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn cost_tier_is_clamped() {
        assert_eq!(CostTier::new(9).0, 3);
        assert_eq!(CostTier::new(0).0, 1);
        assert_eq!(CostTier::new(2).label(), "Balanced");
    }

    #[test]
    fn category_parsing_and_axis() {
        assert_eq!(TaskCategory::parse("Debugging").unwrap(), TaskCategory::Debugging);
        assert_eq!(TaskCategory::Coding.primary_score(), ScoreAxis::Coding);
        assert!(TaskCategory::parse("astrology").is_err());
    }
}
