//! `providers` repository.

use reverie_core::domain::{Provider, Protocol};
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{b, bool_int, i, s};
use crate::error::{StoreError, StoreResult};
use crate::Store;

const COLUMNS: &str = "id, name, protocol, base_url, api_key, search_engine_config_json, is_enabled, created_at";

fn map_row(row: &SqliteRow) -> Provider {
    let protocol_raw = s(row, "protocol");
    let protocol = Protocol::parse(&protocol_raw).unwrap_or(Protocol::OpenAiCompat);
    Provider {
        id: s(row, "id"),
        name: s(row, "name"),
        protocol,
        base_url: s(row, "base_url"),
        api_key: s(row, "api_key"),
        search_engine_config_json: s(row, "search_engine_config_json"),
        is_enabled: b(row, "is_enabled"),
        created_at: reverie_core::time::from_db(&s(row, "created_at")),
    }
}

impl Store {
    /// Every provider, newest last.
    pub async fn list_providers(&self) -> StoreResult<Vec<Provider>> {
        let sql = format!("SELECT {COLUMNS} FROM providers ORDER BY created_at ASC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    /// Providers that are enabled, in creation order.
    pub async fn list_enabled_providers(&self) -> StoreResult<Vec<Provider>> {
        let sql = format!("SELECT {COLUMNS} FROM providers WHERE is_enabled = 1 ORDER BY created_at ASC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn get_provider(&self, id: &str) -> StoreResult<Option<Provider>> {
        let sql = format!("SELECT {COLUMNS} FROM providers WHERE id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    /// Insert or replace a provider.
    pub async fn upsert_provider(&self, provider: &Provider) -> StoreResult<()> {
        provider.validate().map_err(|err| StoreError::invalid(err))?;
        sqlx::query(
            "INSERT INTO providers (id, name, protocol, base_url, api_key, search_engine_config_json, is_enabled, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                protocol = excluded.protocol,
                base_url = excluded.base_url,
                api_key = excluded.api_key,
                search_engine_config_json = excluded.search_engine_config_json,
                is_enabled = excluded.is_enabled",
        )
        .bind(&provider.id)
        .bind(&provider.name)
        .bind(provider.protocol.as_str())
        .bind(&provider.base_url)
        .bind(&provider.api_key)
        .bind(&provider.search_engine_config_json)
        .bind(bool_int(provider.is_enabled))
        .bind(time::to_db(provider.created_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Remove a provider (model profiles cascade).
    pub async fn delete_provider(&self, id: &str) -> StoreResult<()> {
        let result = sqlx::query("DELETE FROM providers WHERE id = ?").bind(id).execute(self.pool()).await?;
        if result.rows_affected() == 0 {
            return Err(StoreError::not_found(format!("provider {id}")));
        }
        Ok(())
    }

    /// Toggle a provider without rewriting every field.
    pub async fn set_provider_enabled(&self, id: &str, enabled: bool) -> StoreResult<()> {
        sqlx::query("UPDATE providers SET is_enabled = ? WHERE id = ?")
            .bind(bool_int(enabled))
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// Update only the search engine credentials blob.
    pub async fn update_provider_search_config(&self, id: &str, json: &str) -> StoreResult<()> {
        sqlx::query("UPDATE providers SET search_engine_config_json = ? WHERE id = ?")
            .bind(json)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// The first enabled provider with at least one model, used as a default.
    pub async fn default_provider(&self) -> StoreResult<Option<Provider>> {
        let providers = self.list_enabled_providers().await?;
        Ok(providers.into_iter().next())
    }

    /// Count of providers, cheap for the settings screen.
    pub async fn count_providers(&self) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM providers").fetch_one(self.pool()).await?;
        Ok(i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> Store {
        Store::connect_in_memory().await.expect("in-memory store")
    }

    #[tokio::test]
    async fn upsert_and_read_provider() {
        let store = store().await;
        let mut provider = Provider::create("OpenAI", Protocol::OpenAiCompat, "https://api.openai.com/v1", "sk-test-1234");
        provider.search_engine_config_json = "{\"tavily_api_key\":\"tvly\"}".to_string();
        store.upsert_provider(&provider).await.unwrap();

        let loaded = store.get_provider(&provider.id).await.unwrap().unwrap();
        assert_eq!(loaded.name, "OpenAI");
        assert_eq!(loaded.protocol, Protocol::OpenAiCompat);
        assert!(loaded.search_config().has_api_backend());
    }

    #[tokio::test]
    async fn upsert_is_idempotent() {
        let store = store().await;
        let mut provider = Provider::create("P", Protocol::Ollama, "http://127.0.0.1:11434", "");
        store.upsert_provider(&provider).await.unwrap();
        provider.name = "Renamed".into();
        store.upsert_provider(&provider).await.unwrap();
        assert_eq!(store.count_providers().await.unwrap(), 1);
        assert_eq!(store.get_provider(&provider.id).await.unwrap().unwrap().name, "Renamed");
    }

    #[tokio::test]
    async fn delete_missing_provider_errors() {
        let store = store().await;
        assert!(store.delete_provider("nope").await.is_err());
    }

    #[tokio::test]
    async fn invalid_provider_is_rejected() {
        let store = store().await;
        let bad = Provider::create("", Protocol::Anthropic, "not-a-url", "");
        assert!(store.upsert_provider(&bad).await.is_err());
    }

    #[tokio::test]
    async fn enabled_filtering_works() {
        let store = store().await;
        let mut a = Provider::create("A", Protocol::Ollama, "http://127.0.0.1:11434", "");
        let b_provider = Provider::create("B", Protocol::Ollama, "http://127.0.0.1:11435", "");
        a.is_enabled = false;
        store.upsert_provider(&a).await.unwrap();
        store.upsert_provider(&b_provider).await.unwrap();
        assert_eq!(store.list_providers().await.unwrap().len(), 2);
        assert_eq!(store.list_enabled_providers().await.unwrap().len(), 1);
    }
}
