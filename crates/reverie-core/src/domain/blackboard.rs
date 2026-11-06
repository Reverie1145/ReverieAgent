//! Shared context blackboard entries.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time;

/// One published finding on the session blackboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlackboardEntry {
    pub id: String,
    pub session_id: String,
    pub source_agent_id: String,
    pub topic_tag: String,
    pub summary: String,
    pub full_content: String,
    /// `["*"]` broadcasts to every agent.
    pub target_agent_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl BlackboardEntry {
    pub fn publish(
        session_id: impl Into<String>,
        source_agent_id: impl Into<String>,
        topic_tag: impl Into<String>,
        summary: impl Into<String>,
        full_content: impl Into<String>,
        target_agent_ids: Vec<String>,
    ) -> Self {
        let targets = if target_agent_ids.is_empty() { vec!["*".to_string()] } else { target_agent_ids };
        BlackboardEntry {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.into(),
            source_agent_id: source_agent_id.into(),
            topic_tag: topic_tag.into(),
            summary: summary.into(),
            full_content: full_content.into(),
            target_agent_ids: targets,
            created_at: time::now(),
        }
    }

    /// Whether this entry is addressed to the given agent.
    pub fn targets(&self, agent_id: &str) -> bool {
        self.target_agent_ids.iter().any(|t| t == "*" || t == agent_id)
    }

    /// Whether a free-text query matches the topic tag, summary or content.
    pub fn matches(&self, query: &str) -> bool {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return true;
        }
        self.topic_tag.to_ascii_lowercase().contains(&needle)
            || self.summary.to_ascii_lowercase().contains(&needle)
            || self.full_content.to_ascii_lowercase().contains(&needle)
    }

    /// Render a compact card for injection into another agent's context.
    pub fn to_context_block(&self) -> String {
        format!(
            "### [{}] from {}\n{}\n\n{}",
            self.topic_tag, self.source_agent_id, self.summary, self.full_content
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_targets_broadcast() {
        let e = BlackboardEntry::publish("s", "a1", "api", "summary", "full", vec![]);
        assert_eq!(e.target_agent_ids, vec!["*".to_string()]);
        assert!(e.targets("anyone"));
    }

    #[test]
    fn targeted_entries_filter() {
        let e = BlackboardEntry::publish("s", "a1", "api", "s", "f", vec!["a2".into()]);
        assert!(e.targets("a2"));
        assert!(!e.targets("a3"));
    }

    #[test]
    fn matching_is_case_insensitive() {
        let e = BlackboardEntry::publish("s", "a1", "Auth Flow", "Uses JWT", "details", vec![]);
        assert!(e.matches("auth"));
        assert!(e.matches("jwt"));
        assert!(!e.matches("graphql"));
        assert!(e.matches(""));
    }

    #[test]
    fn context_block_is_markdown() {
        let e = BlackboardEntry::publish("s", "a1", "api", "sum", "body", vec![]);
        let block = e.to_context_block();
        assert!(block.contains("### [api] from a1"));
        assert!(block.contains("body"));
    }
}
