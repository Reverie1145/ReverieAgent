//! `agent_runs` repository.

use reverie_core::domain::{AgentArchetype, AgentRun, AgentStatus};
use reverie_core::llm::ThinkingLevel;
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{i, os, s, string_array, encode_string_array};
use crate::error::{StoreError, StoreResult};
use crate::Store;

const COLUMNS: &str = "id, session_id, parent_agent_id, role_name, archetype, model_id, thinking_level, \
     task_description, depend_on_agent_ids, attached_file_ids, workspace_dir, git_branch, status, \
     message_history_json, output_artifact, prompt_tokens, completion_tokens, reasoning_tokens, \
     error_message, created_at, finished_at";

fn map_row(row: &SqliteRow) -> AgentRun {
    AgentRun {
        id: s(row, "id"),
        session_id: s(row, "session_id"),
        parent_agent_id: os(row, "parent_agent_id"),
        role_name: s(row, "role_name"),
        archetype: AgentArchetype::parse(&s(row, "archetype")).unwrap_or(AgentArchetype::Generalist),
        model_id: s(row, "model_id"),
        thinking_level: ThinkingLevel::parse(&s(row, "thinking_level")).unwrap_or(ThinkingLevel::Medium),
        task_description: s(row, "task_description"),
        depend_on_agent_ids: string_array(&s(row, "depend_on_agent_ids")),
        attached_file_ids: string_array(&s(row, "attached_file_ids")),
        workspace_dir: s(row, "workspace_dir"),
        git_branch: os(row, "git_branch"),
        status: AgentStatus::parse(&s(row, "status")).unwrap_or(AgentStatus::Queued),
        message_history_json: s(row, "message_history_json"),
        output_artifact: os(row, "output_artifact"),
        prompt_tokens: i(row, "prompt_tokens"),
        completion_tokens: i(row, "completion_tokens"),
        reasoning_tokens: i(row, "reasoning_tokens"),
        error_message: os(row, "error_message"),
        created_at: time::from_db(&s(row, "created_at")),
        finished_at: time::from_db_opt(os(row, "finished_at").as_deref()),
    }
}

impl Store {
    pub async fn insert_agent(&self, run: &AgentRun) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO agent_runs (
                id, session_id, parent_agent_id, role_name, archetype, model_id, thinking_level,
                task_description, depend_on_agent_ids, attached_file_ids, workspace_dir, git_branch,
                status, message_history_json, output_artifact, prompt_tokens, completion_tokens,
                reasoning_tokens, error_message, created_at, finished_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                model_id = excluded.model_id,
                thinking_level = excluded.thinking_level,
                workspace_dir = excluded.workspace_dir,
                git_branch = excluded.git_branch,
                status = excluded.status,
                message_history_json = excluded.message_history_json,
                output_artifact = excluded.output_artifact,
                prompt_tokens = excluded.prompt_tokens,
                completion_tokens = excluded.completion_tokens,
                reasoning_tokens = excluded.reasoning_tokens,
                error_message = excluded.error_message,
                finished_at = excluded.finished_at",
        )
        .bind(&run.id)
        .bind(&run.session_id)
        .bind(&run.parent_agent_id)
        .bind(&run.role_name)
        .bind(run.archetype.as_str())
        .bind(&run.model_id)
        .bind(run.thinking_level.as_str())
        .bind(&run.task_description)
        .bind(encode_string_array(&run.depend_on_agent_ids))
        .bind(encode_string_array(&run.attached_file_ids))
        .bind(&run.workspace_dir)
        .bind(&run.git_branch)
        .bind(run.status.as_str())
        .bind(&run.message_history_json)
        .bind(&run.output_artifact)
        .bind(run.prompt_tokens)
        .bind(run.completion_tokens)
        .bind(run.reasoning_tokens)
        .bind(&run.error_message)
        .bind(time::to_db(run.created_at))
        .bind(run.finished_at.map(time::to_db))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn get_agent(&self, id: &str) -> StoreResult<Option<AgentRun>> {
        let sql = format!("SELECT {COLUMNS} FROM agent_runs WHERE id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    pub async fn list_agents(&self, session_id: &str) -> StoreResult<Vec<AgentRun>> {
        let sql = format!("SELECT {COLUMNS} FROM agent_runs WHERE session_id = ? ORDER BY created_at ASC");
        let rows = sqlx::query(&sql).bind(session_id).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn update_agent_status(
        &self,
        id: &str,
        status: AgentStatus,
        error_message: Option<&str>,
    ) -> StoreResult<()> {
        let finished = if status.is_terminal() { Some(time::now_db()) } else { None };
        sqlx::query("UPDATE agent_runs SET status = ?, error_message = ?, finished_at = COALESCE(?, finished_at) WHERE id = ?")
            .bind(status.as_str())
            .bind(error_message)
            .bind(finished)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Runtime re-route of a single agent's model and thinking depth.
    pub async fn update_agent_route(
        &self,
        id: &str,
        model_id: &str,
        thinking_level: ThinkingLevel,
    ) -> StoreResult<()> {
        let result = sqlx::query("UPDATE agent_runs SET model_id = ?, thinking_level = ? WHERE id = ?")
            .bind(model_id)
            .bind(thinking_level.as_str())
            .bind(id)
            .execute(self.pool())
            .await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found(format!("agent {id}")));
        }
        Ok(())
    }

    pub async fn set_agent_workspace(&self, id: &str, workspace_dir: &str, branch: Option<&str>) -> StoreResult<()> {
        sqlx::query("UPDATE agent_runs SET workspace_dir = ?, git_branch = ? WHERE id = ?")
            .bind(workspace_dir)
            .bind(branch)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Persist the transcript and token counters after each turn.
    pub async fn save_agent_progress(
        &self,
        id: &str,
        history_json: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
        reasoning_tokens: i64,
    ) -> StoreResult<()> {
        sqlx::query(
            "UPDATE agent_runs
             SET message_history_json = ?, prompt_tokens = ?, completion_tokens = ?, reasoning_tokens = ?
             WHERE id = ?",
        )
        .bind(history_json)
        .bind(prompt_tokens)
        .bind(completion_tokens)
        .bind(reasoning_tokens)
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn finish_agent(
        &self,
        id: &str,
        status: AgentStatus,
        output_artifact: Option<&str>,
        error_message: Option<&str>,
    ) -> StoreResult<()> {
        sqlx::query(
            "UPDATE agent_runs SET status = ?, output_artifact = ?, error_message = ?, finished_at = ? WHERE id = ?",
        )
        .bind(status.as_str())
        .bind(output_artifact)
        .bind(error_message)
        .bind(time::now_db())
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Count agents in a session grouped by status. Used by the swarm grid.
    pub async fn agent_status_counts(&self, session_id: &str) -> StoreResult<Vec<(String, i64)>> {
        let rows = sqlx::query("SELECT status, COUNT(*) AS c FROM agent_runs WHERE session_id = ? GROUP BY status")
            .bind(session_id)
            .fetch_all(self.pool())
            .await?;
        Ok(rows.iter().map(|row| (s(row, "status"), i(row, "c"))).collect())
    }

    pub async fn count_agents(&self, session_id: &str) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM agent_runs WHERE session_id = ?")
            .bind(session_id)
            .fetch_one(self.pool())
            .await?;
        Ok(i(&row, "c"))
    }

    /// Total tokens burned by a session, for the summary card.
    pub async fn session_token_totals(&self, session_id: &str) -> StoreResult<(i64, i64, i64)> {
        let row = sqlx::query(
            "SELECT COALESCE(SUM(prompt_tokens),0) AS p, COALESCE(SUM(completion_tokens),0) AS c,
                    COALESCE(SUM(reasoning_tokens),0) AS r
             FROM agent_runs WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_one(self.pool())
        .await?;
        Ok((i(&row, "p"), i(&row, "c"), i(&row, "r")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::{AgentSpec, Session};

    async fn seeded() -> (Store, Session) {
        let store = Store::connect_in_memory().await.unwrap();
        let session = Session::create("t", "F:/ws", "o", "auto", true);
        store.insert_session(&session).await.unwrap();
        (store, session)
    }

    #[tokio::test]
    async fn insert_and_read_agent() {
        let (store, session) = seeded().await;
        let spec = AgentSpec::new("Search-Researcher-012", AgentArchetype::Researcher, "find crates");
        let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        store.insert_agent(&run).await.unwrap();

        let loaded = store.get_agent(&run.id).await.unwrap().unwrap();
        assert_eq!(loaded.role_name, "Search-Researcher-012");
        assert_eq!(loaded.archetype, AgentArchetype::Researcher);
        assert_eq!(loaded.status, AgentStatus::Queued);
    }

    #[tokio::test]
    async fn route_switch_updates_row() {
        let (store, session) = seeded().await;
        let spec = AgentSpec::new("Coder", AgentArchetype::Coder, "write it");
        let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        store.insert_agent(&run).await.unwrap();

        store.update_agent_route(&run.id, "p:model-x", ThinkingLevel::Max).await.unwrap();
        let loaded = store.get_agent(&run.id).await.unwrap().unwrap();
        assert_eq!(loaded.model_id, "p:model-x");
        assert_eq!(loaded.thinking_level, ThinkingLevel::Max);
        assert!(store.update_agent_route("missing", "m", ThinkingLevel::Off).await.is_err());
    }

    #[tokio::test]
    async fn finish_sets_timestamp_and_status() {
        let (store, session) = seeded().await;
        let spec = AgentSpec::new("W", AgentArchetype::Writer, "t");
        let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        store.insert_agent(&run).await.unwrap();

        store.finish_agent(&run.id, AgentStatus::Completed, Some("# done"), None).await.unwrap();
        let loaded = store.get_agent(&run.id).await.unwrap().unwrap();
        assert_eq!(loaded.status, AgentStatus::Completed);
        assert!(loaded.finished_at.is_some());
        assert_eq!(loaded.output_artifact.as_deref(), Some("# done"));
    }

    #[tokio::test]
    async fn progress_and_totals() {
        let (store, session) = seeded().await;
        let spec = AgentSpec::new("A", AgentArchetype::Analyst, "t");
        let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
        store.insert_agent(&run).await.unwrap();
        store.save_agent_progress(&run.id, "[]", 100, 40, 12).await.unwrap();

        let (p, c, r) = store.session_token_totals(&session.id).await.unwrap();
        assert_eq!((p, c, r), (100, 40, 12));
    }

    #[tokio::test]
    async fn status_counts_group_correctly() {
        let (store, session) = seeded().await;
        for i in 0..3 {
            let spec = AgentSpec::new(format!("a{i}"), AgentArchetype::Coder, "t");
            let run = AgentRun::from_spec(&session.id, None, &spec, "F:/ws");
            store.insert_agent(&run).await.unwrap();
            store.update_agent_status(&run.id, AgentStatus::Completed, None).await.unwrap();
        }
        let counts = store.agent_status_counts(&session.id).await.unwrap();
        assert_eq!(counts, vec![("completed".to_string(), 3)]);
        assert_eq!(store.count_agents(&session.id).await.unwrap(), 3);
    }
}
