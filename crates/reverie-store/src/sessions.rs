//! `sessions` repository.

use reverie_core::domain::{ContextStrategy, IsolationStrategy, Session, SessionStatus, TopologyMode};
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{b, bool_int, i, os, s};
use crate::error::{StoreError, StoreResult};
use crate::Store;

const COLUMNS: &str = "id, title, workspace_path, user_objective, thinking_mode_override, web_search_enabled, \
     status, topology_mode, context_strategy, isolation_strategy, final_summary, created_at, updated_at";

fn map_row(row: &SqliteRow) -> Session {
    Session {
        id: s(row, "id"),
        title: s(row, "title"),
        workspace_path: s(row, "workspace_path"),
        user_objective: s(row, "user_objective"),
        thinking_mode_override: s(row, "thinking_mode_override"),
        web_search_enabled: b(row, "web_search_enabled"),
        status: SessionStatus::parse(&s(row, "status")).unwrap_or(SessionStatus::Planning),
        topology_mode: TopologyMode::parse(&s(row, "topology_mode")).unwrap_or(TopologyMode::HierarchicalTree),
        context_strategy: ContextStrategy::parse(&s(row, "context_strategy"))
            .unwrap_or(ContextStrategy::Blackboard),
        isolation_strategy: IsolationStrategy::parse(&s(row, "isolation_strategy"))
            .unwrap_or(IsolationStrategy::SharedVfsLock),
        final_summary: os(row, "final_summary"),
        created_at: time::from_db(&s(row, "created_at")),
        updated_at: time::from_db(&s(row, "updated_at")),
    }
}

impl Store {
    pub async fn insert_session(&self, session: &Session) -> StoreResult<()> {
        session.validate().map_err(|err| StoreError::invalid(err))?;
        sqlx::query(
            "INSERT INTO sessions (
                id, title, workspace_path, user_objective, thinking_mode_override, web_search_enabled,
                status, topology_mode, context_strategy, isolation_strategy, final_summary, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                workspace_path = excluded.workspace_path,
                user_objective = excluded.user_objective,
                thinking_mode_override = excluded.thinking_mode_override,
                web_search_enabled = excluded.web_search_enabled,
                status = excluded.status,
                topology_mode = excluded.topology_mode,
                context_strategy = excluded.context_strategy,
                isolation_strategy = excluded.isolation_strategy,
                final_summary = excluded.final_summary,
                updated_at = excluded.updated_at",
        )
        .bind(&session.id)
        .bind(&session.title)
        .bind(&session.workspace_path)
        .bind(&session.user_objective)
        .bind(&session.thinking_mode_override)
        .bind(bool_int(session.web_search_enabled))
        .bind(session.status.as_str())
        .bind(session.topology_mode.as_str())
        .bind(session.context_strategy.as_str())
        .bind(session.isolation_strategy.as_str())
        .bind(&session.final_summary)
        .bind(time::to_db(session.created_at))
        .bind(time::to_db(session.updated_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> StoreResult<Option<Session>> {
        let sql = format!("SELECT {COLUMNS} FROM sessions WHERE id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    /// Sessions ordered by most recently touched.
    pub async fn list_sessions(&self, limit: i64) -> StoreResult<Vec<Session>> {
        let sql = format!("SELECT {COLUMNS} FROM sessions ORDER BY updated_at DESC LIMIT ?");
        let rows = sqlx::query(&sql).bind(limit.clamp(1, 500)).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn update_session_status(&self, id: &str, status: SessionStatus) -> StoreResult<()> {
        sqlx::query("UPDATE sessions SET status = ?, updated_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(time::now_db())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn update_session_strategy(
        &self,
        id: &str,
        topology: TopologyMode,
        context: ContextStrategy,
        isolation: IsolationStrategy,
    ) -> StoreResult<()> {
        sqlx::query(
            "UPDATE sessions SET topology_mode = ?, context_strategy = ?, isolation_strategy = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(topology.as_str())
        .bind(context.as_str())
        .bind(isolation.as_str())
        .bind(time::now_db())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn update_session_summary(&self, id: &str, summary: &str) -> StoreResult<()> {
        sqlx::query("UPDATE sessions SET final_summary = ?, updated_at = ? WHERE id = ?")
            .bind(summary)
            .bind(time::now_db())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn touch_session(&self, id: &str) -> StoreResult<()> {
        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
            .bind(time::now_db())
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn count_sessions(&self) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM sessions").fetch_one(self.pool()).await?;
        Ok(i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(title: &str) -> Session {
        Session::create(title, "F:/ws", "objective", "auto", true)
    }

    #[tokio::test]
    async fn insert_and_read() {
        let store = Store::connect_in_memory().await.unwrap();
        let session = sample("Build ReverieAgent");
        store.insert_session(&session).await.unwrap();
        let loaded = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.title, "Build ReverieAgent");
        assert!(loaded.web_search_enabled);
        assert_eq!(loaded.status, SessionStatus::Planning);
    }

    #[tokio::test]
    async fn listing_is_newest_first() {
        let store = Store::connect_in_memory().await.unwrap();
        let a = sample("first");
        store.insert_session(&a).await.unwrap();
        let b = sample("second");
        store.insert_session(&b).await.unwrap();
        store.touch_session(&a.id).await.unwrap();
        let list = store.list_sessions(10).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, a.id);
    }

    #[tokio::test]
    async fn status_and_strategy_updates_persist() {
        let store = Store::connect_in_memory().await.unwrap();
        let session = sample("s");
        store.insert_session(&session).await.unwrap();
        store.update_session_status(&session.id, SessionStatus::Running).await.unwrap();
        store
            .update_session_strategy(
                &session.id,
                TopologyMode::DagPipeline,
                ContextStrategy::Hybrid,
                IsolationStrategy::GitWorktree,
            )
            .await
            .unwrap();
        let loaded = store.get_session(&session.id).await.unwrap().unwrap();
        assert_eq!(loaded.status, SessionStatus::Running);
        assert_eq!(loaded.topology_mode, TopologyMode::DagPipeline);
        assert_eq!(loaded.isolation_strategy, IsolationStrategy::GitWorktree);
    }

    #[tokio::test]
    async fn empty_objective_is_rejected() {
        let store = Store::connect_in_memory().await.unwrap();
        let mut session = sample("s");
        session.user_objective = "   ".into();
        assert!(store.insert_session(&session).await.is_err());
    }

    #[tokio::test]
    async fn deleting_session_cascades() {
        let store = Store::connect_in_memory().await.unwrap();
        let session = sample("s");
        store.insert_session(&session).await.unwrap();
        store.delete_session(&session.id).await.unwrap();
        assert_eq!(store.count_sessions().await.unwrap(), 0);
    }
}
