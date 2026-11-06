//! `context_blackboard` repository.

use reverie_core::domain::BlackboardEntry;
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{encode_string_array, s, string_array};
use crate::error::StoreResult;
use crate::Store;

const COLUMNS: &str =
    "id, session_id, source_agent_id, topic_tag, summary, full_content, target_agent_ids, created_at";

fn map_row(row: &SqliteRow) -> BlackboardEntry {
    BlackboardEntry {
        id: s(row, "id"),
        session_id: s(row, "session_id"),
        source_agent_id: s(row, "source_agent_id"),
        topic_tag: s(row, "topic_tag"),
        summary: s(row, "summary"),
        full_content: s(row, "full_content"),
        target_agent_ids: string_array(&s(row, "target_agent_ids")),
        created_at: time::from_db(&s(row, "created_at")),
    }
}

impl Store {
    pub async fn insert_blackboard(&self, entry: &BlackboardEntry) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO context_blackboard (
                id, session_id, source_agent_id, topic_tag, summary, full_content, target_agent_ids, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                summary = excluded.summary,
                full_content = excluded.full_content,
                target_agent_ids = excluded.target_agent_ids",
        )
        .bind(&entry.id)
        .bind(&entry.session_id)
        .bind(&entry.source_agent_id)
        .bind(&entry.topic_tag)
        .bind(&entry.summary)
        .bind(&entry.full_content)
        .bind(encode_string_array(&entry.target_agent_ids))
        .bind(time::to_db(entry.created_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_blackboard(&self, session_id: &str, limit: i64) -> StoreResult<Vec<BlackboardEntry>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM context_blackboard WHERE session_id = ? ORDER BY created_at DESC LIMIT ?"
        );
        let rows = sqlx::query(&sql)
            .bind(session_id)
            .bind(limit.clamp(1, 2000))
            .fetch_all(self.pool())
            .await?;
        Ok(rows.iter().map(map_row).collect())
    }

    /// Full-text-ish search across tag, summary and content.
    pub async fn query_blackboard(
        &self,
        session_id: &str,
        needle: &str,
        limit: i64,
    ) -> StoreResult<Vec<BlackboardEntry>> {
        let pattern = format!("%{}%", needle.trim());
        let sql = format!(
            "SELECT {COLUMNS} FROM context_blackboard
             WHERE session_id = ? AND (topic_tag LIKE ? OR summary LIKE ? OR full_content LIKE ?)
             ORDER BY created_at DESC LIMIT ?"
        );
        let rows = sqlx::query(&sql)
            .bind(session_id)
            .bind(&pattern)
            .bind(&pattern)
            .bind(&pattern)
            .bind(limit.clamp(1, 500))
            .fetch_all(self.pool())
            .await?;
        Ok(rows.iter().map(map_row).collect())
    }

    /// Entries addressed to a specific agent (plus broadcasts).
    pub async fn blackboard_for_agent(&self, session_id: &str, agent_id: &str) -> StoreResult<Vec<BlackboardEntry>> {
        let all = self.list_blackboard(session_id, 2000).await?;
        Ok(all.into_iter().filter(|entry| entry.targets(agent_id)).collect())
    }

    pub async fn count_blackboard(&self, session_id: &str) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM context_blackboard WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(self.pool())
            .await?;
        Ok(crate::db::i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::Session;

    async fn seeded() -> (Store, Session) {
        let store = Store::connect_in_memory().await.unwrap();
        let session = Session::create("t", "F:/ws", "o", "auto", true);
        store.insert_session(&session).await.unwrap();
        (store, session)
    }

    #[tokio::test]
    async fn publish_and_query() {
        let (store, session) = seeded().await;
        let entry = BlackboardEntry::publish(&session.id, "a1", "auth", "JWT chosen", "Full notes on JWT", vec![]);
        store.insert_blackboard(&entry).await.unwrap();

        let all = store.list_blackboard(&session.id, 10).await.unwrap();
        assert_eq!(all.len(), 1);
        let found = store.query_blackboard(&session.id, "jwt", 10).await.unwrap();
        assert_eq!(found.len(), 1);
        let missing = store.query_blackboard(&session.id, "graphql", 10).await.unwrap();
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn agent_filtering_respects_targets() {
        let (store, session) = seeded().await;
        store
            .insert_blackboard(&BlackboardEntry::publish(&session.id, "a1", "t1", "s", "c", vec![]))
            .await
            .unwrap();
        store
            .insert_blackboard(&BlackboardEntry::publish(&session.id, "a2", "t2", "s", "c", vec!["a9".into()]))
            .await
            .unwrap();

        let for_a9 = store.blackboard_for_agent(&session.id, "a9").await.unwrap();
        assert_eq!(for_a9.len(), 2);
        let for_a8 = store.blackboard_for_agent(&session.id, "a8").await.unwrap();
        assert_eq!(for_a8.len(), 1);
    }

    #[tokio::test]
    async fn cascade_on_session_delete() {
        let (store, session) = seeded().await;
        store
            .insert_blackboard(&BlackboardEntry::publish(&session.id, "a1", "t", "s", "c", vec![]))
            .await
            .unwrap();
        assert_eq!(store.count_blackboard(&session.id).await.unwrap(), 1);
        store.delete_session(&session.id).await.unwrap();
        assert_eq!(store.count_blackboard(&session.id).await.unwrap(), 0);
    }
}
