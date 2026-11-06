//! Self-evolution records: versioned role prompts and reusable playbooks.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};
use crate::time;

/// Kind of evolution artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvolutionRecordType {
    ArchetypePrompt,
    PlaybookSkill,
}

impl EvolutionRecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            EvolutionRecordType::ArchetypePrompt => "archetype_prompt",
            EvolutionRecordType::PlaybookSkill => "playbook_skill",
        }
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "archetype_prompt" | "prompt" => Ok(EvolutionRecordType::ArchetypePrompt),
            "playbook_skill" | "playbook" | "skill" => Ok(EvolutionRecordType::PlaybookSkill),
            other => Err(CoreError::InvalidEnum { kind: "EvolutionRecordType", value: other.to_string() }),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EvolutionRecordType::ArchetypePrompt => "Role prompt",
            EvolutionRecordType::PlaybookSkill => "Playbook",
        }
    }
}

/// A versioned prompt or playbook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionRecord {
    pub id: String,
    pub record_type: EvolutionRecordType,
    pub key_name: String,
    pub title: String,
    pub content: String,
    pub version: i64,
    pub changelog: String,
    pub updated_at: DateTime<Utc>,
}

impl EvolutionRecord {
    pub fn new(
        record_type: EvolutionRecordType,
        key_name: impl Into<String>,
        title: impl Into<String>,
        content: impl Into<String>,
        changelog: impl Into<String>,
    ) -> Self {
        EvolutionRecord {
            id: Uuid::new_v4().to_string(),
            record_type,
            key_name: key_name.into(),
            title: title.into(),
            content: content.into(),
            version: 1,
            changelog: changelog.into(),
            updated_at: time::now(),
        }
    }

    /// Bump the version and record why.
    pub fn evolve(&mut self, content: impl Into<String>, changelog: impl Into<String>) {
        self.content = content.into();
        self.changelog = changelog.into();
        self.version += 1;
        self.updated_at = time::now();
    }

    /// Only replace the body when it actually changed; returns whether a new
    /// version was written.
    pub fn evolve_if_changed(&mut self, content: &str, changelog: &str) -> bool {
        if self.content.trim() == content.trim() {
            return false;
        }
        self.evolve(content, changelog);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_roundtrip() {
        assert_eq!(
            EvolutionRecordType::parse("playbook").unwrap(),
            EvolutionRecordType::PlaybookSkill
        );
        assert!(EvolutionRecordType::parse("poem").is_err());
        assert_eq!(EvolutionRecordType::ArchetypePrompt.label(), "Role prompt");
    }

    #[test]
    fn evolve_bumps_version() {
        let mut record = EvolutionRecord::new(
            EvolutionRecordType::PlaybookSkill,
            "playbook:rust-audit",
            "Rust audit",
            "v1 body",
            "initial",
        );
        assert!(record.evolve_if_changed("v2 body", "refined"));
        assert_eq!(record.version, 2);
        assert!(!record.evolve_if_changed("v2 body", "noop"));
        assert_eq!(record.version, 2);
    }
}
