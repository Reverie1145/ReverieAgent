//! The continuous self-evolution engine.
//!
//! After every session it folds empirical performance back into the model
//! matrix, distils the winning strategy into a reusable playbook, and appends
//! lessons to the role prompts that underperformed.

use std::collections::BTreeMap;
use std::sync::Arc;

use reverie_core::domain::{
    AgentArchetype, AgentRun, AgentStatus, EvolutionRecord, EvolutionRecordType, ModelProfile, Session,
};
use reverie_core::events::{EventSink, SystemEvent};
use reverie_core::llm::{ChatMessage, LlmRequest, ThinkingLevel};
use reverie_core::time;
use reverie_core::util::slugify;
use reverie_llm::LlmGateway;
use reverie_store::Store;
use serde::{Deserialize, Serialize};

use crate::prompt::default_prompt;

/// Summary of one reflection pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvolutionReport {
    pub session_id: String,
    pub models_updated: usize,
    pub playbooks_written: usize,
    pub prompts_evolved: usize,
    pub notes: Vec<String>,
}

/// Post-mortem reflection.
pub struct EvolutionEngine {
    store: Arc<Store>,
    gateway: Arc<LlmGateway>,
}

/// Per-model empirical rollup for one session.
#[derive(Debug, Default, Clone, Copy)]
struct ModelStats {
    total: i64,
    success: i64,
    latency_total_ms: i64,
}

impl EvolutionEngine {
    pub fn new(store: Arc<Store>, gateway: Arc<LlmGateway>) -> Self {
        EvolutionEngine { store, gateway }
    }

    /// Ensure every archetype has a versioned prompt row.
    pub async fn seed_prompts(&self) -> Result<usize, reverie_store::StoreError> {
        let archetypes = [
            AgentArchetype::MetaOrchestrator,
            AgentArchetype::ContextBroker,
            AgentArchetype::EnvironmentWarden,
            AgentArchetype::Researcher,
            AgentArchetype::Coder,
            AgentArchetype::Reviewer,
            AgentArchetype::Analyst,
            AgentArchetype::Writer,
            AgentArchetype::VisionInspector,
            AgentArchetype::DataExtractor,
            AgentArchetype::MergeResolver,
            AgentArchetype::Generalist,
        ];

        let mut created = 0usize;
        for archetype in archetypes {
            let key = archetype.prompt_key();
            if self.store.get_evolution_by_key(&key).await?.is_some() {
                continue;
            }
            let record = EvolutionRecord::new(
                EvolutionRecordType::ArchetypePrompt,
                key,
                archetype.label(),
                default_prompt(&archetype),
                "seed prompt",
            );
            self.store.upsert_evolution_record(&record).await?;
            created += 1;
        }
        Ok(created)
    }

    /// Reflect on a finished session.
    pub async fn reflect_session(
        &self,
        session_id: &str,
        sink: &dyn EventSink,
    ) -> Result<EvolutionReport, reverie_store::StoreError> {
        let mut report = EvolutionReport { session_id: session_id.to_string(), ..Default::default() };

        let Some(session) = self.store.get_session(session_id).await? else {
            report.notes.push("session not found".to_string());
            return Ok(report);
        };
        let agents = self.store.list_agents(session_id).await?;
        if agents.is_empty() {
            report.notes.push("no agents to learn from".to_string());
            return Ok(report);
        }

        report.models_updated = self.update_model_statistics(&agents).await?;
        report.playbooks_written = self.write_playbook(&session, &agents).await?;
        report.prompts_evolved = self.evolve_prompts(&agents).await?;

        report.notes.push(format!(
            "{} agents, {} completed, {} failed",
            agents.len(),
            agents.iter().filter(|agent| agent.status == AgentStatus::Completed).count(),
            agents.iter().filter(|agent| agent.status == AgentStatus::Failed).count()
        ));

        sink.emit(SystemEvent::LogMessage {
            level: "info".to_string(),
            scope: "EvolutionEngine".to_string(),
            message: format!(
                "reflection complete: {} models updated, {} playbook(s), {} prompt(s) evolved",
                report.models_updated, report.playbooks_written, report.prompts_evolved
            ),
        });
        Ok(report)
    }

    async fn update_model_statistics(&self, agents: &[AgentRun]) -> Result<usize, reverie_store::StoreError> {
        let mut rollup: BTreeMap<String, ModelStats> = BTreeMap::new();
        for agent in agents {
            if agent.model_id.is_empty() {
                continue;
            }
            let entry = rollup.entry(agent.model_id.clone()).or_default();
            entry.total += 1;
            if agent.status == AgentStatus::Completed {
                entry.success += 1;
                entry.latency_total_ms += agent.elapsed_ms();
            }
        }

        let mut updated = 0usize;
        for (model_id, stats) in rollup {
            let Some(mut profile) = self.store.get_model_profile(&model_id).await? else {
                continue;
            };
            if stats.total == 0 {
                continue;
            }
            if stats.success > 0 {
                profile.observe_latency(stats.latency_total_ms / stats.success);
            }
            let rate = stats.success as f64 / stats.total as f64;
            let delta = (rate - 0.75) * 0.4;
            profile.reasoning_score = (profile.reasoning_score + delta).clamp(0.0, 10.0);
            profile.coding_score = (profile.coding_score + delta).clamp(0.0, 10.0);
            profile.extraction_score = (profile.extraction_score + delta).clamp(0.0, 10.0);
            profile.updated_at = time::now();
            self.store.upsert_model_profile(&profile).await?;
            updated += 1;
        }
        Ok(updated)
    }

    /// Ask a strong model to distil the playbook; fall back to a deterministic
    /// template when no model is reachable.
    async fn distill_playbook(&self, session: &Session, agents: &[AgentRun]) -> Option<String> {
        let profiles = self.store.list_model_profiles().await.ok()?;
        let best: &ModelProfile = profiles
            .iter()
            .filter(|profile| profile.supports_tools)
            .max_by(|a, b| a.reasoning_score.partial_cmp(&b.reasoning_score).unwrap_or(std::cmp::Ordering::Equal))?;
        let provider = self.store.get_provider(&best.provider_id).await.ok()??;
        if !provider.is_enabled {
            return None;
        }

        let roster = agents
            .iter()
            .map(|agent| {
                format!(
                    "- {} ({}) model={} think={} status={}",
                    agent.role_name,
                    agent.archetype.as_str(),
                    agent.model_id,
                    agent.thinking_level.as_str(),
                    agent.status.as_str()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "You are the ReverieAgent evolution engine. Distil the session below into a reusable \
             playbook that a future run of a similar task could follow.\n\n\
             Objective: {}\nTopology: {}\nContext strategy: {}\nIsolation: {}\n\nRoster:\n{roster}\n\n\
             Write Markdown with these sections: When to use, Topology, Agent roster, Sequencing, \
             Verification, Pitfalls. Be concrete and under 400 words. No preamble.",
            session.user_objective.trim(),
            session.topology_mode.as_str(),
            session.context_strategy.as_str(),
            session.isolation_strategy.as_str()
        );

        let request = LlmRequest::new(
            &best.model_name,
            vec![
                ChatMessage::system("You write terse, actionable engineering playbooks."),
                ChatMessage::user(prompt),
            ],
        )
        .with_thinking(ThinkingLevel::Low)
        .with_limits(Some(1200), Some(0.2));

        match self.gateway.complete_with_retry(&provider, request).await {
            Ok(completion) if !completion.text.trim().is_empty() => Some(completion.text),
            _ => None,
        }
    }

    async fn write_playbook(&self, session: &Session, agents: &[AgentRun]) -> Result<usize, reverie_store::StoreError> {
        let key = format!("playbook:{}", slugify(&session.title, 60));
        let distilled = self.distill_playbook(session, agents).await;

        let body = distilled.unwrap_or_else(|| deterministic_playbook(session, agents));
        let title = format!("Playbook — {}", session.title);

        match self.store.get_evolution_by_key(&key).await? {
            Some(mut existing) => {
                if existing.evolve_if_changed(&body, "refined from a new session run") {
                    self.store.upsert_evolution_record(&existing).await?;
                }
                Ok(1)
            }
            None => {
                let record = EvolutionRecord::new(
                    EvolutionRecordType::PlaybookSkill,
                    key,
                    title,
                    body,
                    "distilled from a completed session",
                );
                self.store.upsert_evolution_record(&record).await?;
                Ok(1)
            }
        }
    }

    /// Append a lesson to the prompts of archetypes that failed.
    async fn evolve_prompts(&self, agents: &[AgentRun]) -> Result<usize, reverie_store::StoreError> {
        let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for agent in agents {
            if agent.status != AgentStatus::Failed {
                continue;
            }
            let lesson = agent
                .error_message
                .clone()
                .unwrap_or_else(|| "failed without a recorded error".to_string());
            failures
                .entry(agent.archetype.prompt_key())
                .or_default()
                .push(reverie_core::util::truncate_chars(lesson.trim(), 240));
        }

        let mut evolved = 0usize;
        for (key, lessons) in failures {
            let Some(mut record) = self.store.get_evolution_by_key(&key).await? else {
                continue;
            };
            let addition = lessons
                .iter()
                .map(|lesson| format!("- {lesson}"))
                .collect::<Vec<_>>()
                .join("\n");
            let section = format!("\n\n## Lessons from past runs\n{addition}\n");
            let content = if record.content.contains("## Lessons from past runs") {
                format!("{}{}", record.content, addition)
            } else {
                format!("{}{}", record.content, section)
            };
            if record.evolve_if_changed(&content, &format!("absorbed {} failure(s)", lessons.len())) {
                self.store.upsert_evolution_record(&record).await?;
                evolved += 1;
            }
        }
        Ok(evolved)
    }
}

/// A deterministic playbook used when no model is available.
fn deterministic_playbook(session: &Session, agents: &[AgentRun]) -> String {
    let roster = agents
        .iter()
        .map(|agent| {
            format!(
                "| {} | {} | {} | {} |",
                agent.role_name,
                agent.archetype.label(),
                if agent.model_id.is_empty() { "auto".to_string() } else { agent.model_id.clone() },
                agent.status.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "# Playbook — {}\n\n\
         ## When to use\n{}\n\n\
         ## Topology\n- Mode: {}\n- Context: {}\n- Isolation: {}\n\n\
         ## Agent roster\n| Role | Archetype | Model | Outcome |\n| --- | --- | --- | --- |\n{roster}\n\n\
         ## Sequencing\nAgents ran under the `{}` topology with `{}` context routing. Dependencies were \
         declared explicitly, so work that could not start early was queued rather than guessed at.\n\n\
         ## Verification\nEvery agent reported a concrete artifact; failures were surfaced with their \
         error message instead of being silently retried.\n",
        session.title,
        session.user_objective.trim(),
        session.topology_mode.as_str(),
        session.context_strategy.as_str(),
        session.isolation_strategy.as_str(),
        session.topology_mode.as_str(),
        session.context_strategy.as_str()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::AgentSpec;
    use reverie_core::events::NoopSink;

    async fn engine() -> (EvolutionEngine, Arc<Store>) {
        let store = Arc::new(Store::connect_in_memory().await.unwrap());
        let gateway = Arc::new(LlmGateway::new().unwrap());
        (EvolutionEngine::new(store.clone(), gateway), store)
    }

    #[tokio::test]
    async fn seeding_is_idempotent() {
        let (engine, store) = engine().await;
        let first = engine.seed_prompts().await.unwrap();
        assert_eq!(first, 12);
        let second = engine.seed_prompts().await.unwrap();
        assert_eq!(second, 0);
        assert_eq!(store.count_evolution_records().await.unwrap(), 12);
    }

    #[tokio::test]
    async fn reflection_writes_a_playbook_and_updates_models() {
        let (engine, store) = engine().await;
        engine.seed_prompts().await.unwrap();

        let provider = reverie_core::domain::Provider::create(
            "p",
            reverie_core::domain::Protocol::OpenAiCompat,
            "https://api.example.com/v1",
            "sk-test-1234",
        );
        store.upsert_provider(&provider).await.unwrap();
        let mut profile = ModelProfile::provisional(&provider.id, "gpt-4o");
        profile.latency_ms = 1000;
        store.upsert_model_profile(&profile).await.unwrap();

        let session = Session::create("Refactor auth", "F:/ws", "make auth safe", "auto", true);
        store.insert_session(&session).await.unwrap();

        let spec = AgentSpec::new("Auth-Dev", AgentArchetype::Coder, "refactor");
        let mut run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        run.model_id = profile.id.clone();
        store.insert_agent(&run).await.unwrap();
        store
            .finish_agent(&run.id, AgentStatus::Completed, Some("done"), None)
            .await
            .unwrap();

        let report = engine.reflect_session(&session.id, &NoopSink).await.unwrap();
        assert_eq!(report.models_updated, 1);
        assert_eq!(report.playbooks_written, 1);

        let playbook = store.get_evolution_by_key("playbook:refactor-auth").await.unwrap();
        assert!(playbook.is_some());
        assert!(playbook.unwrap().content.contains("Auth-Dev"));
    }

    #[tokio::test]
    async fn failures_evolve_the_role_prompt() {
        let (engine, store) = engine().await;
        engine.seed_prompts().await.unwrap();

        let session = Session::create("Broken build", "F:/ws", "fix it", "auto", false);
        store.insert_session(&session).await.unwrap();
        let spec = AgentSpec::new("Coder-1", AgentArchetype::Coder, "fix");
        let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        store.insert_agent(&run).await.unwrap();
        store
            .finish_agent(&run.id, AgentStatus::Failed, None, Some("cargo check failed"))
            .await
            .unwrap();

        let report = engine.reflect_session(&session.id, &NoopSink).await.unwrap();
        assert_eq!(report.prompts_evolved, 1);

        let record = store.get_evolution_by_key("archetype:coder").await.unwrap().unwrap();
        assert!(record.content.contains("Lessons from past runs"));
        assert!(record.content.contains("cargo check failed"));
        assert_eq!(record.version, 2);
    }

    #[tokio::test]
    async fn missing_session_is_reported() {
        let (engine, _store) = engine().await;
        let report = engine.reflect_session("nope", &NoopSink).await.unwrap();
        assert!(report.notes[0].contains("not found"));
    }

    #[test]
    fn deterministic_playbook_has_required_sections() {
        let session = Session::create("T", "F:/ws", "obj", "auto", true);
        let spec = AgentSpec::new("R", AgentArchetype::Researcher, "t");
        let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        let text = deterministic_playbook(&session, &[run]);
        for section in ["## When to use", "## Topology", "## Agent roster", "## Verification"] {
            assert!(text.contains(section), "missing {section}");
        }
    }
}
