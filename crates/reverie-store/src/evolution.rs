//! `evolution_store` repository: versioned prompts and playbooks.

use reverie_core::domain::{EvolutionRecord, EvolutionRecordType};
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{i, s};
use crate::error::{StoreError, StoreResult};
use crate::Store;

const COLUMNS: &str = "id, record_type, key_name, title, content, version, changelog, updated_at";

fn map_row(row: &SqliteRow) -> EvolutionRecord {
    EvolutionRecord {
        id: s(row, "id"),
        record_type: EvolutionRecordType::parse(&s(row, "record_type"))
            .unwrap_or(EvolutionRecordType::PlaybookSkill),
        key_name: s(row, "key_name"),
        title: s(row, "title"),
        content: s(row, "content"),
        version: i(row, "version"),
        changelog: s(row, "changelog"),
        updated_at: time::from_db(&s(row, "updated_at")),
    }
}

impl Store {
    pub async fn list_evolution_records(&self) -> StoreResult<Vec<EvolutionRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM evolution_store ORDER BY record_type ASC, key_name ASC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn list_evolution_by_type(&self, record_type: EvolutionRecordType) -> StoreResult<Vec<EvolutionRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM evolution_store WHERE record_type = ? ORDER BY key_name ASC");
        let rows = sqlx::query(&sql).bind(record_type.as_str()).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn get_evolution_record(&self, id: &str) -> StoreResult<Option<EvolutionRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM evolution_store WHERE id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    pub async fn get_evolution_by_key(&self, key_name: &str) -> StoreResult<Option<EvolutionRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM evolution_store WHERE key_name = ?");
        let row = sqlx::query(&sql).bind(key_name).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    /// Insert, or update an existing key by bumping its version.
    pub async fn upsert_evolution_record(&self, record: &EvolutionRecord) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO evolution_store (id, record_type, key_name, title, content, version, changelog, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(key_name) DO UPDATE SET
                title = excluded.title,
                content = excluded.content,
                version = excluded.version,
                changelog = excluded.changelog,
                updated_at = excluded.updated_at",
        )
        .bind(&record.id)
        .bind(record.record_type.as_str())
        .bind(&record.key_name)
        .bind(&record.title)
        .bind(&record.content)
        .bind(record.version)
        .bind(&record.changelog)
        .bind(time::to_db(record.updated_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Replace content, bump the version and record a changelog entry.
    pub async fn evolve_record_content(
        &self,
        id: &str,
        content: &str,
        changelog: &str,
    ) -> StoreResult<EvolutionRecord> {
        let mut record = self
            .get_evolution_record(id)
            .await?
            .ok_or_else(|| StoreError::not_found(format!("evolution record {id}")))?;
        record.evolve(content, changelog);
        self.upsert_evolution_record(&record).await?;
        Ok(record)
    }

    /// Read a role prompt by archetype key, if the engine has evolved one.
    pub async fn archetype_prompt(&self, archetype: &str) -> StoreResult<Option<String>> {
        let key = format!("archetype:{archetype}");
        Ok(self.get_evolution_by_key(&key).await?.map(|record| record.content))
    }

    pub async fn count_evolution_records(&self) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM evolution_store").fetch_one(self.pool()).await?;
        Ok(i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(key: &str, body: &str) -> EvolutionRecord {
        EvolutionRecord::new(
            EvolutionRecordType::PlaybookSkill,
            key,
            "Rust audit playbook",
            body,
            "initial distillation",
        )
    }

    #[tokio::test]
    async fn insert_and_list() {
        let store = Store::connect_in_memory().await.unwrap();
        store.upsert_evolution_record(&sample("playbook:rust", "step 1")).await.unwrap();
        let all = store.list_evolution_records().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].version, 1);
    }

    #[tokio::test]
    async fn upsert_bumps_version_on_same_key() {
        let store = Store::connect_in_memory().await.unwrap();
        let mut record = sample("playbook:rust", "step 1");
        store.upsert_evolution_record(&record).await.unwrap();
        record.evolve("step 2", "refined");
        store.upsert_evolution_record(&record).await.unwrap();

        assert_eq!(store.count_evolution_records().await.unwrap(), 1);
        let loaded = store.get_evolution_by_key("playbook:rust").await.unwrap().unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.content, "step 2");
    }

    #[tokio::test]
    async fn evolve_by_id_requires_existing() {
        let store = Store::connect_in_memory().await.unwrap();
        assert!(store.evolve_record_content("missing", "x", "y").await.is_err());

        let record = sample("playbook:a", "body");
        store.upsert_evolution_record(&record).await.unwrap();
        let evolved = store.evolve_record_content(&record.id, "body v2", "improved").await.unwrap();
        assert_eq!(evolved.version, 2);
        assert_eq!(evolved.changelog, "improved");
    }

    #[tokio::test]
    async fn archetype_prompt_lookup() {
        let store = Store::connect_in_memory().await.unwrap();
        store
            .upsert_evolution_record(&EvolutionRecord::new(
                EvolutionRecordType::ArchetypePrompt,
                "archetype:coder",
                "Coder",
                "You are a precise Rust engineer.",
                "seed",
            ))
            .await
            .unwrap();
        assert!(store.archetype_prompt("coder").await.unwrap().is_some());
        assert!(store.archetype_prompt("poet").await.unwrap().is_none());
    }
}
