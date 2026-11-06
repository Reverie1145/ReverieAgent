//! The Context Broker: decides exactly what a sub-agent sees at start-up.
//!
//! Hundreds of agents cannot each carry the full transcript, so the broker
//! assembles a budgeted briefing: the objective, mounted attachments, upstream
//! deliverables, and the blackboard slice that matters for this agent's role.

use reverie_core::domain::{AgentRun, ContextStrategy, Session};
use reverie_store::{Store, StoreResult};

/// Inputs for one briefing.
pub struct BrokerRequest<'a> {
    pub session: &'a Session,
    pub run: &'a AgentRun,
    pub strategy: ContextStrategy,
    /// Character budget for the whole briefing.
    pub max_chars: usize,
}

/// Per-item truncation applied before the global budget.
const UPSTREAM_CHARS: usize = 8_000;
const ATTACHMENT_PREVIEW_CHARS: usize = 4_000;
const BLACKBOARD_ENTRIES: i64 = 12;

/// Assembles briefings.
pub struct ContextBrokerAgent;

impl ContextBrokerAgent {
    /// Build the briefing block for one agent.
    pub async fn build_context(store: &Store, request: BrokerRequest<'_>) -> StoreResult<String> {
        let session = request.session;
        let run = request.run;
        let mut out = String::new();

        out.push_str(&format!("# Briefing for {}\n\n", run.role_name));
        out.push_str("## Session objective\n");
        out.push_str(session.user_objective.trim());
        out.push_str("\n\n");

        // Mounted attachments.
        if !run.attached_file_ids.is_empty() {
            let attachments = store.get_attachments(&run.attached_file_ids).await?;
            if !attachments.is_empty() {
                out.push_str("## Mounted files\n");
                for attachment in &attachments {
                    out.push_str(&format!("- {} [{}]\n", attachment.describe(), attachment.id));
                    if let Some(preview) = &attachment.extracted_text_preview {
                        let trimmed = reverie_core::util::truncate_middle(preview, ATTACHMENT_PREVIEW_CHARS);
                        out.push_str(&format!("  ```\n  {}\n  ```\n", trimmed.replace('\n', "\n  ")));
                    }
                }
                out.push('\n');
            }
        }

        // Upstream deliverables.
        if !run.depend_on_agent_ids.is_empty() {
            out.push_str("## Upstream deliverables\n");
            for dependency in &run.depend_on_agent_ids {
                if let Some(upstream) = store.get_agent(dependency).await? {
                    let body = upstream.output_artifact.unwrap_or_else(|| {
                        format!("(no artifact — status: {})", upstream.status.as_str())
                    });
                    let budget = match request.strategy {
                        ContextStrategy::PeerToPeer => UPSTREAM_CHARS * 2,
                        ContextStrategy::Hybrid => UPSTREAM_CHARS,
                        _ => UPSTREAM_CHARS / 2,
                    };
                    out.push_str(&format!(
                        "### {} ({})\n{}\n\n",
                        upstream.role_name,
                        upstream.archetype.label(),
                        reverie_core::util::truncate_middle(&body, budget)
                    ));
                }
            }
        }

        // Blackboard slice.
        if request.strategy.uses_blackboard() {
            let mut entries = store.blackboard_for_agent(&session.id, &run.id).await?;
            entries.retain(|entry| entry.source_agent_id != run.id);
            entries.truncate(BLACKBOARD_ENTRIES as usize);
            if !entries.is_empty() {
                out.push_str("## Shared findings\n");
                for entry in &entries {
                    out.push_str(&format!(
                        "- **[{}]** {} — {}\n",
                        entry.topic_tag,
                        entry.summary,
                        reverie_core::util::truncate_chars(entry.full_content.trim(), 600)
                    ));
                }
                out.push('\n');
            }
        }

        // Peer roster, so the agent knows who else exists.
        let roster = store.list_agents(&session.id).await?;
        let peers: Vec<String> = roster
            .iter()
            .filter(|peer| peer.id != run.id)
            .take(24)
            .map(|peer| format!("- {} ({}, {})", peer.role_name, peer.archetype.label(), peer.status.as_str()))
            .collect();
        if !peers.is_empty() {
            out.push_str("## Swarm roster\n");
            out.push_str(&peers.join("\n"));
            out.push_str("\n\n");
        }

        out.push_str("## Your task\n");
        out.push_str(run.task_description.trim());
        out.push('\n');

        let budget = if request.max_chars == 0 { 24_000 } else { request.max_chars };
        Ok(reverie_core::util::truncate_middle(&out, budget))
    }

    /// Budget derived from the model's context window: leave room for tools and
    /// the reply.
    pub fn context_budget(context_window: i64, thinking_share: f64) -> usize {
        let usable = (context_window as f64) * (0.5 - thinking_share.clamp(0.0, 0.45));
        usable.clamp(4_000.0, 160_000.0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::{AgentArchetype, AgentSpec, BlackboardEntry};

    async fn seeded() -> (Store, Session, AgentRun, AgentRun) {
        let store = Store::connect_in_memory().await.unwrap();
        let session = Session::create("t", "F:/ws", "ship the feature", "auto", true);
        store.insert_session(&session).await.unwrap();

        let upstream_spec = AgentSpec::new("Researcher-1", AgentArchetype::Researcher, "find sources");
        let upstream = AgentRun::from_spec(&session.id, None, &upstream_spec, "F:/ws");
        store.insert_agent(&upstream).await.unwrap();
        store
            .finish_agent(&upstream.id, reverie_core::domain::AgentStatus::Completed, Some("Sources: A, B"), None)
            .await
            .unwrap();

        let mut spec = AgentSpec::new("Coder-1", AgentArchetype::Coder, "implement the API");
        spec.depend_on_agent_ids = vec![upstream.id.clone()];
        let worker = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        store.insert_agent(&worker).await.unwrap();

        (store, session, upstream, worker)
    }

    #[tokio::test]
    async fn briefing_contains_objective_and_task() {
        let (store, session, _upstream, worker) = seeded().await;
        let briefing = ContextBrokerAgent::build_context(
            &store,
            BrokerRequest { session: &session, run: &worker, strategy: ContextStrategy::Blackboard, max_chars: 40_000 },
        )
        .await
        .unwrap();
        assert!(briefing.contains("ship the feature"));
        assert!(briefing.contains("implement the API"));
        assert!(briefing.contains("Coder-1"));
    }

    #[tokio::test]
    async fn upstream_deliverables_are_included() {
        let (store, session, _upstream, worker) = seeded().await;
        let briefing = ContextBrokerAgent::build_context(
            &store,
            BrokerRequest { session: &session, run: &worker, strategy: ContextStrategy::Hybrid, max_chars: 40_000 },
        )
        .await
        .unwrap();
        assert!(briefing.contains("Sources: A, B"));
        assert!(briefing.contains("Researcher-1"));
    }

    #[tokio::test]
    async fn blackboard_entries_are_routed() {
        let (store, session, _upstream, worker) = seeded().await;
        let entry = BlackboardEntry::publish(
            &session.id,
            "someone-else",
            "api-contract",
            "REST with cursor pagination",
            "Full contract details",
            vec!["*".into()],
        );
        store.insert_blackboard(&entry).await.unwrap();

        let briefing = ContextBrokerAgent::build_context(
            &store,
            BrokerRequest { session: &session, run: &worker, strategy: ContextStrategy::Blackboard, max_chars: 40_000 },
        )
        .await
        .unwrap();
        assert!(briefing.contains("REST with cursor pagination"));
    }

    #[tokio::test]
    async fn peer_to_peer_skips_the_blackboard() {
        let (store, session, _upstream, worker) = seeded().await;
        let entry = BlackboardEntry::publish(&session.id, "x", "t", "SHOULD NOT APPEAR", "c", vec![]);
        store.insert_blackboard(&entry).await.unwrap();

        let briefing = ContextBrokerAgent::build_context(
            &store,
            BrokerRequest { session: &session, run: &worker, strategy: ContextStrategy::PeerToPeer, max_chars: 40_000 },
        )
        .await
        .unwrap();
        assert!(!briefing.contains("SHOULD NOT APPEAR"));
    }

    #[tokio::test]
    async fn budget_is_respected() {
        let (store, session, _upstream, worker) = seeded().await;
        let briefing = ContextBrokerAgent::build_context(
            &store,
            BrokerRequest { session: &session, run: &worker, strategy: ContextStrategy::Hybrid, max_chars: 400 },
        )
        .await
        .unwrap();
        assert!(briefing.chars().count() <= 420);
    }

    #[test]
    fn context_budget_scales_with_window() {
        assert_eq!(ContextBrokerAgent::context_budget(2048, 0.0), 4_000);
        assert!(ContextBrokerAgent::context_budget(200_000, 0.2) > 50_000);
        // A heavy thinking budget sharply reduces the room left for context.
        assert!(ContextBrokerAgent::context_budget(1_000_000, 0.5) < 60_000);
    }
}
