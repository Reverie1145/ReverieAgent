//! Bootstrap model profiler: pull the live roster, probe it concurrently, then
//! let an evaluation agent turn the raw signal into usable scores.

pub mod evaluate;
pub mod heuristics;
pub mod probe;

use std::sync::Arc;

use futures::stream::{self, StreamExt};
use reverie_core::domain::{ModelProfile, Provider};
use reverie_core::events::{EventSink, SystemEvent};
use reverie_core::llm::ThinkingLevel;
use reverie_core::time;
use reverie_store::{Store, StoreError};

use crate::error::{LlmError, LlmResult};
use crate::gateway::LlmGateway;
use crate::profiler::evaluate::{evaluate_models, EvaluationInput, EvaluationScore};
use crate::profiler::probe::{CapabilityProbe, ProbeReport};

/// Default ceiling on how many models are live-probed in one run. Providers can
/// expose hundreds of models; probing all of them would be abusive.
pub const DEFAULT_PROBE_LIMIT: usize = 40;

/// Concurrent probe fan-out.
const PROBE_CONCURRENCY: usize = 4;

fn store_err(err: StoreError) -> LlmError {
    LlmError::Other(format!("store failure: {err}"))
}

/// Coordinates discovery, probing and evaluation.
#[derive(Clone)]
pub struct ModelProfilerService {
    gateway: Arc<LlmGateway>,
    store: Arc<Store>,
    probe_limit: usize,
}

impl ModelProfilerService {
    pub fn new(gateway: Arc<LlmGateway>, store: Arc<Store>) -> Self {
        ModelProfilerService { gateway, store, probe_limit: DEFAULT_PROBE_LIMIT }
    }

    pub fn with_probe_limit(mut self, limit: usize) -> Self {
        self.probe_limit = limit.max(1);
        self
    }

    async fn provider(&self, provider_id: &str) -> LlmResult<Provider> {
        self.store
            .get_provider(provider_id)
            .await
            .map_err(store_err)?
            .ok_or_else(|| LlmError::Other(format!("provider {provider_id} not found")))
    }

    /// Pull the live model list and upsert one row per model, preserving any
    /// previously measured scores.
    pub async fn pull_models(&self, provider_id: &str, sink: &dyn EventSink) -> LlmResult<Vec<ModelProfile>> {
        let provider = self.provider(provider_id).await?;
        sink.emit(SystemEvent::ProfilingProgress {
            provider_id: provider_id.to_string(),
            stage: "pull".to_string(),
            message: format!("Querying {} for its model list…", provider.name),
            completed: 0,
            total: 0,
        });

        let remote = self.gateway.list_models(&provider).await?;
        let existing = self.store.list_models_for_provider(provider_id).await.map_err(store_err)?;

        let mut out = Vec::with_capacity(remote.len());
        for (index, item) in remote.iter().enumerate() {
            let id = ModelProfile::compose_id(provider_id, &item.model_name);
            let mut profile = match existing.iter().find(|candidate| candidate.id == id) {
                Some(previous) => previous.clone(),
                None => heuristics::to_profile(provider_id, item, provider.protocol),
            };
            if let Some(context) = item.context_window.filter(|value| *value > 0) {
                profile.context_window = context;
            }
            profile.updated_at = time::now();

            self.store.upsert_model_profile(&profile).await.map_err(store_err)?;
            sink.emit(SystemEvent::ModelProfileUpdated { model_id: profile.id.clone() });
            sink.emit(SystemEvent::ProfilingProgress {
                provider_id: provider_id.to_string(),
                stage: "pull".to_string(),
                message: format!("Discovered {}", profile.model_name),
                completed: index + 1,
                total: remote.len(),
            });
            out.push(profile);
        }

        Ok(out)
    }

    /// Probe and evaluate every known model for a provider.
    pub async fn profile_provider(&self, provider_id: &str, sink: &dyn EventSink) -> LlmResult<Vec<ModelProfile>> {
        let provider = self.provider(provider_id).await?;
        let mut models = self.store.list_models_for_provider(provider_id).await.map_err(store_err)?;

        if models.is_empty() {
            models = self.pull_models(provider_id, sink).await?;
        }
        if models.is_empty() {
            return Err(LlmError::NoModel(format!(
                "provider `{}` reported no models; check the base URL and credentials",
                provider.name
            )));
        }

        models.truncate(self.probe_limit);
        let total = models.len();

        sink.emit(SystemEvent::ProfilingProgress {
            provider_id: provider_id.to_string(),
            stage: "probe".to_string(),
            message: format!("Probing {total} models concurrently…"),
            completed: 0,
            total,
        });

        let probe = CapabilityProbe::new(&self.gateway);
        let results: Vec<(ModelProfile, ProbeReport)> = stream::iter(models.into_iter().map(|model| {
            let probe = &probe;
            let provider = &provider;
            async move {
                let report = probe.run(provider, &model.model_name).await;
                (model, report)
            }
        }))
        .buffer_unordered(PROBE_CONCURRENCY)
        .collect()
        .await;

        let mut updated: Vec<ModelProfile> = Vec::with_capacity(results.len());
        for (index, (mut profile, report)) in results.into_iter().enumerate() {
            apply_probe(&mut profile, &report);
            profile.updated_at = time::now();
            sink.emit(SystemEvent::ProfilingProgress {
                provider_id: provider_id.to_string(),
                stage: "probe".to_string(),
                message: format!(
                    "{}: {} ms, vision {}, thinking {}",
                    profile.model_name,
                    profile.latency_ms,
                    yes_no(profile.supports_vision),
                    yes_no(profile.supports_thinking)
                ),
                completed: index + 1,
                total,
            });
            updated.push(profile);
        }

        // Evaluation agent pass.
        let evaluator = updated
            .iter()
            .filter(|profile| profile.supports_tools)
            .max_by(|a, b| a.reasoning_score.partial_cmp(&b.reasoning_score).unwrap_or(std::cmp::Ordering::Equal))
            .map(|profile| profile.model_name.clone());

        if let Some(evaluator_model) = evaluator {
            sink.emit(SystemEvent::ProfilingProgress {
                provider_id: provider_id.to_string(),
                stage: "evaluate".to_string(),
                message: format!("Evaluation agent ({evaluator_model}) is scoring the roster…"),
                completed: 0,
                total: 1,
            });

            let inputs: Vec<EvaluationInput> = updated
                .iter()
                .map(|profile| EvaluationInput {
                    model_name: profile.model_name.clone(),
                    context_window: profile.context_window,
                    supports_vision: profile.supports_vision,
                    supports_thinking: profile.supports_thinking,
                    latency_ms: profile.latency_ms,
                    thinking_levels: profile
                        .supported_thinking_levels
                        .iter()
                        .map(|level| level.as_str().to_string())
                        .collect(),
                })
                .collect();

            match evaluate_models(&self.gateway, &provider, &evaluator_model, &inputs).await {
                Ok(scores) => apply_scores(&mut updated, &scores),
                Err(err) => {
                    sink.emit(SystemEvent::ProfilingProgress {
                        provider_id: provider_id.to_string(),
                        stage: "evaluate".to_string(),
                        message: format!("Evaluation skipped: {err}"),
                        completed: 1,
                        total: 1,
                    });
                }
            }
        }

        for profile in &updated {
            self.store.upsert_model_profile(profile).await.map_err(store_err)?;
            sink.emit(SystemEvent::ModelProfileUpdated { model_id: profile.id.clone() });
        }

        sink.emit(SystemEvent::ProfilingProgress {
            provider_id: provider_id.to_string(),
            stage: "done".to_string(),
            message: format!("Profiled {} models.", updated.len()),
            completed: total,
            total,
        });

        Ok(updated)
    }

    /// Pull and profile every enabled provider.
    pub async fn profile_all(&self, sink: &dyn EventSink) -> LlmResult<Vec<ModelProfile>> {
        let providers = self.store.list_enabled_providers().await.map_err(store_err)?;
        let mut all = Vec::new();
        for provider in providers {
            match self.profile_provider(&provider.id, sink).await {
                Ok(mut profiles) => all.append(&mut profiles),
                Err(err) => {
                    sink.emit(SystemEvent::ProfilingProgress {
                        provider_id: provider.id.clone(),
                        stage: "error".to_string(),
                        message: format!("{} failed: {err}", provider.name),
                        completed: 0,
                        total: 0,
                    });
                }
            }
        }
        Ok(all)
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

/// Fold a probe report into a profile, keeping heuristics where the probe was
/// inconclusive.
pub fn apply_probe(profile: &mut ModelProfile, report: &ProbeReport) {
    if !report.probed {
        return;
    }
    if report.latency_ms > 0 {
        profile.observe_latency(report.latency_ms);
    }
    if let Some(value) = report.supports_tools {
        profile.supports_tools = value;
    }
    if let Some(value) = report.supports_vision {
        profile.supports_vision = value;
    }
    if let Some(value) = report.supports_thinking {
        profile.supports_thinking = value;
    }
    if let Some(levels) = &report.supported_thinking_levels {
        if !levels.is_empty() {
            profile.supported_thinking_levels = levels.clone();
        }
    }
    if !profile.supports_thinking {
        profile.supported_thinking_levels = vec![ThinkingLevel::Off];
        profile.default_thinking_level = ThinkingLevel::Off;
    } else if !profile.supported_thinking_levels.contains(&profile.default_thinking_level)
        || profile.default_thinking_level == ThinkingLevel::Off
    {
        // A thinking-capable model should default to a real reasoning depth,
        // clamped to whatever the probe actually accepted.
        profile.default_thinking_level = ThinkingLevel::Medium.clamp_to(&profile.supported_thinking_levels);
    }
}

/// Apply evaluation-agent scores, matching by model name.
pub fn apply_scores(profiles: &mut [ModelProfile], scores: &[EvaluationScore]) {
    for profile in profiles.iter_mut() {
        if let Some(score) = scores.iter().find(|candidate| candidate.model_name == profile.model_name) {
            profile.reasoning_score = score.reasoning_score;
            profile.coding_score = score.coding_score;
            profile.extraction_score = score.extraction_score;
            profile.cost_tier = reverie_core::domain::CostTier::new(score.cost_tier);
            profile.task_fit_notes = score.task_fit_notes.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::CostTier;

    fn profile() -> ModelProfile {
        let mut model = ModelProfile::provisional("p", "m");
        model.latency_ms = 1000;
        model
    }

    #[test]
    fn probe_overrides_capabilities() {
        let mut model = profile();
        let report = ProbeReport {
            latency_ms: 2000,
            supports_tools: Some(false),
            supports_vision: Some(true),
            supports_thinking: Some(true),
            supported_thinking_levels: Some(vec![ThinkingLevel::Off, ThinkingLevel::Low]),
            errors: vec![],
            probed: true,
        };
        apply_probe(&mut model, &report);
        assert!(!model.supports_tools);
        assert!(model.supports_vision);
        assert!(model.supports_thinking);
        assert_eq!(model.supported_thinking_levels.len(), 2);
        assert_eq!(model.latency_ms, 1300);
        assert_eq!(model.default_thinking_level, ThinkingLevel::Low);
    }

    #[test]
    fn failed_probe_keeps_heuristics() {
        let mut model = profile();
        model.supports_vision = true;
        let report = ProbeReport { errors: vec!["timeout".into()], probed: false, ..Default::default() };
        apply_probe(&mut model, &report);
        assert!(model.supports_vision);
        assert_eq!(model.latency_ms, 1000);
    }

    #[test]
    fn non_thinking_models_are_normalized() {
        let mut model = profile();
        model.supports_thinking = true;
        model.supported_thinking_levels = ThinkingLevel::all().to_vec();
        let report = ProbeReport {
            supports_thinking: Some(false),
            supported_thinking_levels: Some(vec![ThinkingLevel::Off]),
            probed: true,
            latency_ms: 10,
            ..Default::default()
        };
        apply_probe(&mut model, &report);
        assert_eq!(model.default_thinking_level, ThinkingLevel::Off);
        assert_eq!(model.supported_thinking_levels, vec![ThinkingLevel::Off]);
    }

    #[test]
    fn scores_are_matched_by_name() {
        let mut profiles = vec![profile()];
        let scores = vec![EvaluationScore {
            model_name: "m".into(),
            reasoning_score: 9.1,
            coding_score: 8.2,
            extraction_score: 3.3,
            cost_tier: 3,
            task_fit_notes: "frontier".into(),
        }];
        apply_scores(&mut profiles, &scores);
        assert_eq!(profiles[0].reasoning_score, 9.1);
        assert_eq!(profiles[0].cost_tier, CostTier(3));
        assert_eq!(profiles[0].task_fit_notes, "frontier");
    }

    #[test]
    fn unmatched_scores_are_ignored() {
        let mut profiles = vec![profile()];
        let before = profiles[0].reasoning_score;
        apply_scores(
            &mut profiles,
            &[EvaluationScore {
                model_name: "other".into(),
                reasoning_score: 1.0,
                coding_score: 1.0,
                extraction_score: 1.0,
                cost_tier: 1,
                task_fit_notes: String::new(),
            }],
        );
        assert_eq!(profiles[0].reasoning_score, before);
    }
}
