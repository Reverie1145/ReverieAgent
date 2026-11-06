//! Dynamic model and thinking-depth routing.
//!
//! The router is pure: it takes the current capability matrix and a task
//! description, and returns the best (model, thinking level) pair. All policy
//! lives here rather than in the agent loop so it can be unit tested and tuned.

use reverie_core::domain::{ModelProfile, TaskCategory};
use reverie_core::llm::ThinkingLevel;

use crate::error::{LlmError, LlmResult};

/// What the caller needs.
#[derive(Debug, Clone)]
pub struct RouteRequest {
    pub category: TaskCategory,
    /// 0.0 = trivial, 1.0 = hardest thing in the session.
    pub complexity: f32,
    pub requires_vision: bool,
    pub requires_tools: bool,
    /// A hard floor on context window, when the caller knows it.
    pub min_context: Option<i64>,
    /// `Some` when the user or orchestrator pinned a depth.
    pub session_override: Option<ThinkingLevel>,
}

impl RouteRequest {
    pub fn new(category: TaskCategory, complexity: f32) -> Self {
        RouteRequest {
            category,
            complexity: complexity.clamp(0.0, 1.0),
            requires_vision: false,
            requires_tools: true,
            min_context: None,
            session_override: None,
        }
    }

    pub fn with_vision(mut self, required: bool) -> Self {
        self.requires_vision = required;
        self
    }

    pub fn with_tools(mut self, required: bool) -> Self {
        self.requires_tools = required;
        self
    }

    pub fn with_context_floor(mut self, tokens: Option<i64>) -> Self {
        self.min_context = tokens;
        self
    }

    pub fn with_override(mut self, level: Option<ThinkingLevel>) -> Self {
        self.session_override = level;
        self
    }
}

/// The chosen model plus the depth it should think at.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    pub profile: ModelProfile,
    pub thinking: ThinkingLevel,
    pub score: f64,
    pub rationale: String,
}

/// Stateless routing policy.
pub struct ModelRouter;

impl ModelRouter {
    /// Choose a model and thinking depth for a task.
    pub fn select(profiles: &[ModelProfile], request: &RouteRequest) -> LlmResult<RouteDecision> {
        if profiles.is_empty() {
            return Err(LlmError::NoModel("no model profiles have been configured".into()));
        }

        let candidates: Vec<&ModelProfile> = profiles
            .iter()
            .filter(|profile| !request.requires_vision || profile.supports_vision)
            .filter(|profile| !request.requires_tools || profile.supports_tools)
            .filter(|profile| {
                request
                    .min_context
                    .map(|min| profile.effective_context_window() >= min)
                    .unwrap_or(true)
            })
            .collect();

        // Relax the context floor before giving up: a slightly small window is
        // better than refusing to run.
        let candidates = if candidates.is_empty() && request.min_context.is_some() {
            profiles
                .iter()
                .filter(|profile| !request.requires_vision || profile.supports_vision)
                .filter(|profile| !request.requires_tools || profile.supports_tools)
                .collect::<Vec<_>>()
        } else {
            candidates
        };

        if candidates.is_empty() {
            let reason = if request.requires_vision {
                "no configured model accepts image input"
            } else if request.requires_tools {
                "no configured model supports tool calling"
            } else {
                "no configured model matches the request"
            };
            return Err(LlmError::NoModel(reason.to_string()));
        }

        let mut best: Option<(f64, &ModelProfile)> = None;
        for profile in &candidates {
            let value = Self::score(profile, request);
            match best {
                Some((best_score, _)) if best_score >= value => {}
                _ => best = Some((value, profile)),
            }
        }

        let (score, profile) = best.expect("candidates is non-empty");
        let desired = request
            .session_override
            .unwrap_or_else(|| Self::default_thinking(request.category, request.complexity));
        let thinking = profile.resolve_thinking(desired);

        let rationale = format!(
            "{} scored {:.2} for {} (complexity {:.2}); thinking {} → {} (supported: {})",
            profile.model_name,
            score,
            request.category.as_str(),
            request.complexity,
            desired.as_str(),
            thinking.as_str(),
            profile
                .supported_thinking_levels
                .iter()
                .map(|level| level.as_str())
                .collect::<Vec<_>>()
                .join("/")
        );

        Ok(RouteDecision { profile: profile.clone(), thinking, score, rationale })
    }

    /// Preference-ordered score for one model. Higher is better.
    pub fn score(profile: &ModelProfile, request: &RouteRequest) -> f64 {
        let axis = request.category.primary_score();
        let quality = profile.score_for(axis).clamp(0.0, 10.0);
        let reliability = profile.success_rate() * 10.0;
        let latency = (1.0 - (profile.latency_ms.max(0) as f64 / 8000.0).min(1.0)) * 10.0;

        let mut total = quality * 0.62 + reliability * 0.18 + latency * 0.20;

        total += match request.category {
            TaskCategory::Extraction | TaskCategory::Summarization => match profile.cost_tier.0 {
                1 => 1.6,
                2 => 0.4,
                _ => -1.8,
            },
            TaskCategory::Architecture | TaskCategory::Orchestration | TaskCategory::Debugging => {
                match profile.cost_tier.0 {
                    1 => -1.2,
                    2 => 0.0,
                    _ => 0.9,
                }
            }
            _ => match profile.cost_tier.0 {
                1 => 0.2,
                2 => 0.3,
                _ => 0.0,
            },
        };

        // Hard problems reward raw reasoning quality much more than speed.
        if request.complexity > 0.65 {
            total += (quality - 5.0) * 0.25;
        }
        // Very easy, very parallel work should not burn a flagship model.
        if request.complexity < 0.2 && profile.cost_tier.0 == 3 {
            total -= 0.8;
        }
        if let Some(min) = request.min_context {
            if profile.effective_context_window() < min {
                total -= 3.0;
            }
        }
        if request.requires_vision && profile.supports_vision {
            total += 0.6;
        }

        total
    }

    /// Depth the router would pick on its own for a category and complexity.
    pub fn default_thinking(category: TaskCategory, complexity: f32) -> ThinkingLevel {
        let complexity = complexity.clamp(0.0, 1.0);
        match category {
            TaskCategory::Orchestration | TaskCategory::Architecture => {
                if complexity > 0.7 {
                    ThinkingLevel::Max
                } else {
                    ThinkingLevel::High
                }
            }
            TaskCategory::Debugging => {
                if complexity > 0.5 {
                    ThinkingLevel::High
                } else {
                    ThinkingLevel::Medium
                }
            }
            TaskCategory::Coding => {
                if complexity > 0.6 {
                    ThinkingLevel::High
                } else {
                    ThinkingLevel::Medium
                }
            }
            TaskCategory::Research => {
                if complexity > 0.7 {
                    ThinkingLevel::Medium
                } else {
                    ThinkingLevel::Low
                }
            }
            TaskCategory::Extraction => ThinkingLevel::Off,
            TaskCategory::VisionInspection => {
                if complexity > 0.6 {
                    ThinkingLevel::High
                } else {
                    ThinkingLevel::Medium
                }
            }
            TaskCategory::Summarization => ThinkingLevel::Low,
            TaskCategory::General => ThinkingLevel::Medium,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::CostTier;

    fn profile(name: &str, reasoning: f64, coding: f64, extraction: f64, tier: i64, latency: i64) -> ModelProfile {
        let mut model = ModelProfile::provisional("p", name);
        model.reasoning_score = reasoning;
        model.coding_score = coding;
        model.extraction_score = extraction;
        model.cost_tier = CostTier::new(tier);
        model.latency_ms = latency;
        model.supports_thinking = true;
        model.supported_thinking_levels = ThinkingLevel::all().to_vec();
        model
    }

    #[test]
    fn empty_matrix_errors() {
        let err = ModelRouter::select(&[], &RouteRequest::new(TaskCategory::Coding, 0.5)).err().unwrap();
        assert!(matches!(err, LlmError::NoModel(_)));
    }

    #[test]
    fn architecture_prefers_reasoning_flagship() {
        let profiles = vec![
            profile("cheap", 4.0, 4.0, 8.0, 1, 200),
            profile("flagship", 9.5, 9.0, 6.0, 3, 3000),
        ];
        let decision =
            ModelRouter::select(&profiles, &RouteRequest::new(TaskCategory::Architecture, 0.9)).unwrap();
        assert_eq!(decision.profile.model_name, "flagship");
        assert_eq!(decision.thinking, ThinkingLevel::Max);
    }

    #[test]
    fn extraction_prefers_cheap_fast_worker() {
        let profiles = vec![
            profile("cheap", 4.0, 4.0, 8.5, 1, 150),
            profile("flagship", 9.5, 9.0, 7.0, 3, 3000),
        ];
        let decision = ModelRouter::select(&profiles, &RouteRequest::new(TaskCategory::Extraction, 0.1)).unwrap();
        assert_eq!(decision.profile.model_name, "cheap");
        assert_eq!(decision.thinking, ThinkingLevel::Off);
    }

    #[test]
    fn vision_requirement_filters_candidates() {
        let mut text_only = profile("text", 9.0, 9.0, 9.0, 3, 100);
        text_only.supports_vision = false;
        let mut seer = profile("seer", 6.0, 6.0, 6.0, 2, 800);
        seer.supports_vision = true;

        let decision = ModelRouter::select(
            &[text_only, seer],
            &RouteRequest::new(TaskCategory::VisionInspection, 0.5).with_vision(true),
        )
        .unwrap();
        assert_eq!(decision.profile.model_name, "seer");
    }

    #[test]
    fn vision_without_capable_model_errors() {
        let mut text_only = profile("text", 9.0, 9.0, 9.0, 3, 100);
        text_only.supports_vision = false;
        let err = ModelRouter::select(
            &[text_only],
            &RouteRequest::new(TaskCategory::VisionInspection, 0.5).with_vision(true),
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("image"));
    }

    #[test]
    fn tools_requirement_filters_candidates() {
        let mut no_tools = profile("no-tools", 9.0, 9.0, 9.0, 3, 100);
        no_tools.supports_tools = false;
        let decision = ModelRouter::select(
            &[no_tools, profile("tools", 5.0, 5.0, 5.0, 2, 500)],
            &RouteRequest::new(TaskCategory::Coding, 0.4).with_tools(true),
        )
        .unwrap();
        assert_eq!(decision.profile.model_name, "tools");
    }

    #[test]
    fn override_is_clamped_to_model_support() {
        let mut limited = profile("limited", 5.0, 5.0, 5.0, 2, 400);
        limited.supported_thinking_levels = vec![ThinkingLevel::Off, ThinkingLevel::Low];
        let decision = ModelRouter::select(
            &[limited],
            &RouteRequest::new(TaskCategory::Coding, 0.5).with_override(Some(ThinkingLevel::Max)),
        )
        .unwrap();
        assert_eq!(decision.thinking, ThinkingLevel::Low);
    }

    #[test]
    fn non_thinking_model_gets_off() {
        let mut plain = profile("plain", 5.0, 5.0, 5.0, 2, 400);
        plain.supports_thinking = false;
        let decision =
            ModelRouter::select(&[plain], &RouteRequest::new(TaskCategory::Architecture, 0.9)).unwrap();
        assert_eq!(decision.thinking, ThinkingLevel::Off);
    }

    #[test]
    fn context_floor_is_preferred_then_relaxed() {
        let mut small = profile("small", 6.0, 6.0, 6.0, 2, 200);
        small.context_window = 8192;
        let mut large = profile("large", 6.0, 6.0, 6.0, 2, 200);
        large.context_window = 200_000;

        let decision = ModelRouter::select(
            &[small.clone(), large.clone()],
            &RouteRequest::new(TaskCategory::Summarization, 0.3).with_context_floor(Some(100_000)),
        )
        .unwrap();
        assert_eq!(decision.profile.model_name, "large");

        // With only a small model available the floor is relaxed rather than failing.
        let relaxed = ModelRouter::select(
            &[small],
            &RouteRequest::new(TaskCategory::Summarization, 0.3).with_context_floor(Some(100_000)),
        )
        .unwrap();
        assert_eq!(relaxed.profile.model_name, "small");
    }

    #[test]
    fn default_thinking_table_is_sensible() {
        assert_eq!(ModelRouter::default_thinking(TaskCategory::Extraction, 0.9), ThinkingLevel::Off);
        assert_eq!(ModelRouter::default_thinking(TaskCategory::Orchestration, 0.1), ThinkingLevel::High);
        assert_eq!(ModelRouter::default_thinking(TaskCategory::Orchestration, 0.95), ThinkingLevel::Max);
        assert_eq!(ModelRouter::default_thinking(TaskCategory::Summarization, 0.5), ThinkingLevel::Low);
    }

    #[test]
    fn rationale_mentions_model_and_depth() {
        let decision = ModelRouter::select(
            &[profile("m1", 7.0, 7.0, 7.0, 2, 500)],
            &RouteRequest::new(TaskCategory::Coding, 0.5),
        )
        .unwrap();
        assert!(decision.rationale.contains("m1"));
        assert!(decision.rationale.contains("thinking"));
    }
}
