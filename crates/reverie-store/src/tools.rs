//! `dynamic_tools` repository: the runtime self-forged tool registry.

use reverie_core::domain::{DynamicToolRecord, Interpreter};
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{b, bool_int, i, os, s};
use crate::error::StoreResult;
use crate::Store;

const COLUMNS: &str = "id, tool_name, description, parameters_json_schema, interpreter, script_path, \
     created_by_agent_id, is_verified, usage_count, created_at";

fn map_row(row: &SqliteRow) -> DynamicToolRecord {
    DynamicToolRecord {
        id: s(row, "id"),
        tool_name: s(row, "tool_name"),
        description: s(row, "description"),
        parameters_json_schema: s(row, "parameters_json_schema"),
        interpreter: Interpreter::parse(&s(row, "interpreter")).unwrap_or(Interpreter::Bash),
        script_path: s(row, "script_path"),
        created_by_agent_id: os(row, "created_by_agent_id"),
        is_verified: b(row, "is_verified"),
        usage_count: i(row, "usage_count"),
        created_at: time::from_db(&s(row, "created_at")),
    }
}

impl Store {
    /// Register (or overwrite) a forged tool by unique name.
    pub async fn upsert_dynamic_tool(&self, record: &DynamicToolRecord) -> StoreResult<()> {
        sqlx::query(
            "INSERT INTO dynamic_tools (
                id, tool_name, description, parameters_json_schema, interpreter, script_path,
                created_by_agent_id, is_verified, usage_count, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(tool_name) DO UPDATE SET
                description = excluded.description,
                parameters_json_schema = excluded.parameters_json_schema,
                interpreter = excluded.interpreter,
                script_path = excluded.script_path,
                is_verified = excluded.is_verified",
        )
        .bind(&record.id)
        .bind(&record.tool_name)
        .bind(&record.description)
        .bind(&record.parameters_json_schema)
        .bind(record.interpreter.as_str())
        .bind(&record.script_path)
        .bind(&record.created_by_agent_id)
        .bind(bool_int(record.is_verified))
        .bind(record.usage_count)
        .bind(time::to_db(record.created_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn list_dynamic_tools(&self) -> StoreResult<Vec<DynamicToolRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM dynamic_tools ORDER BY created_at DESC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn list_verified_dynamic_tools(&self) -> StoreResult<Vec<DynamicToolRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM dynamic_tools WHERE is_verified = 1 ORDER BY created_at DESC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn get_dynamic_tool(&self, tool_name: &str) -> StoreResult<Option<DynamicToolRecord>> {
        let sql = format!("SELECT {COLUMNS} FROM dynamic_tools WHERE tool_name = ?");
        let row = sqlx::query(&sql).bind(tool_name).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    pub async fn set_dynamic_tool_verified(&self, tool_name: &str, verified: bool) -> StoreResult<()> {
        sqlx::query("UPDATE dynamic_tools SET is_verified = ? WHERE tool_name = ?")
            .bind(bool_int(verified))
            .bind(tool_name)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn increment_tool_usage(&self, tool_name: &str) -> StoreResult<()> {
        sqlx::query("UPDATE dynamic_tools SET usage_count = usage_count + 1 WHERE tool_name = ?")
            .bind(tool_name)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn delete_dynamic_tool(&self, tool_name: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM dynamic_tools WHERE tool_name = ?").bind(tool_name).execute(self.pool()).await?;
        Ok(())
    }

    pub async fn count_dynamic_tools(&self) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM dynamic_tools").fetch_one(self.pool()).await?;
        Ok(i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> DynamicToolRecord {
        DynamicToolRecord::new(
            name,
            "counts lines",
            "{\"type\":\"object\",\"properties\":{\"path\":{\"type\":\"string\"}}}",
            Interpreter::Python3,
            "/tmp/wc.py",
            Some("agent-7".into()),
        )
    }

    #[tokio::test]
    async fn register_and_lookup() {
        let store = Store::connect_in_memory().await.unwrap();
        let mut record = sample("count_lines");
        record.is_verified = true;
        store.upsert_dynamic_tool(&record).await.unwrap();

        let loaded = store.get_dynamic_tool("count_lines").await.unwrap().unwrap();
        assert_eq!(loaded.interpreter, Interpreter::Python3);
        assert!(loaded.is_verified);
        assert_eq!(store.count_dynamic_tools().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn name_is_unique_and_upsert_replaces() {
        let store = Store::connect_in_memory().await.unwrap();
        store.upsert_dynamic_tool(&sample("t")).await.unwrap();
        let mut second = sample("t");
        second.description = "updated".into();
        store.upsert_dynamic_tool(&second).await.unwrap();
        assert_eq!(store.count_dynamic_tools().await.unwrap(), 1);
        assert_eq!(store.get_dynamic_tool("t").await.unwrap().unwrap().description, "updated");
    }

    #[tokio::test]
    async fn usage_counter_increments() {
        let store = Store::connect_in_memory().await.unwrap();
        store.upsert_dynamic_tool(&sample("t")).await.unwrap();
        store.increment_tool_usage("t").await.unwrap();
        store.increment_tool_usage("t").await.unwrap();
        assert_eq!(store.get_dynamic_tool("t").await.unwrap().unwrap().usage_count, 2);
    }

    #[tokio::test]
    async fn verification_filter() {
        let store = Store::connect_in_memory().await.unwrap();
        let mut verified = sample("a");
        verified.is_verified = true;
        store.upsert_dynamic_tool(&verified).await.unwrap();
        store.upsert_dynamic_tool(&sample("b")).await.unwrap();
        assert_eq!(store.list_verified_dynamic_tools().await.unwrap().len(), 1);
        assert_eq!(store.list_dynamic_tools().await.unwrap().len(), 2);
    }
}
