//! `model_profiles` repository.

use reverie_core::domain::{CostTier, ModelProfile};
use reverie_core::llm::ThinkingLevel;
use reverie_core::time;
use crate::db::SqliteRow;

use crate::db::{b, bool_int, f, i, s};
use crate::error::{StoreError, StoreResult};
use crate::Store;

const COLUMNS: &str = "id, provider_id, model_name, context_window, supports_tools, supports_vision, \
     supports_thinking, supported_thinking_levels, default_thinking_level, latency_ms, reasoning_score, \
     coding_score, extraction_score, cost_tier, total_calls, success_calls, task_fit_notes, updated_at";

fn map_row(row: &SqliteRow) -> ModelProfile {
    let supported_raw = s(row, "supported_thinking_levels");
    let default_raw = s(row, "default_thinking_level");
    ModelProfile {
        id: s(row, "id"),
        provider_id: s(row, "provider_id"),
        model_name: s(row, "model_name"),
        context_window: i(row, "context_window"),
        supports_tools: b(row, "supports_tools"),
        supports_vision: b(row, "supports_vision"),
        supports_thinking: b(row, "supports_thinking"),
        supported_thinking_levels: ThinkingLevel::parse_list(&supported_raw),
        default_thinking_level: ThinkingLevel::parse(&default_raw).unwrap_or(ThinkingLevel::Off),
        latency_ms: i(row, "latency_ms"),
        reasoning_score: f(row, "reasoning_score"),
        coding_score: f(row, "coding_score"),
        extraction_score: f(row, "extraction_score"),
        cost_tier: CostTier::new(i(row, "cost_tier")),
        total_calls: i(row, "total_calls"),
        success_calls: i(row, "success_calls"),
        task_fit_notes: s(row, "task_fit_notes"),
        updated_at: time::from_db(&s(row, "updated_at")),
    }
}

impl Store {
    pub async fn list_model_profiles(&self) -> StoreResult<Vec<ModelProfile>> {
        let sql = format!("SELECT {COLUMNS} FROM model_profiles ORDER BY provider_id ASC, model_name ASC");
        let rows = sqlx::query(&sql).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn list_models_for_provider(&self, provider_id: &str) -> StoreResult<Vec<ModelProfile>> {
        let sql = format!("SELECT {COLUMNS} FROM model_profiles WHERE provider_id = ? ORDER BY model_name ASC");
        let rows = sqlx::query(&sql).bind(provider_id).fetch_all(self.pool()).await?;
        Ok(rows.iter().map(map_row).collect())
    }

    pub async fn get_model_profile(&self, id: &str) -> StoreResult<Option<ModelProfile>> {
        let sql = format!("SELECT {COLUMNS} FROM model_profiles WHERE id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(self.pool()).await?;
        Ok(row.as_ref().map(map_row))
    }

    pub async fn upsert_model_profile(&self, profile: &ModelProfile) -> StoreResult<()> {
        if profile.model_name.trim().is_empty() {
            return Err(StoreError::invalid("model_name must not be empty"));
        }
        sqlx::query(
            "INSERT INTO model_profiles (
                id, provider_id, model_name, context_window, supports_tools, supports_vision,
                supports_thinking, supported_thinking_levels, default_thinking_level, latency_ms,
                reasoning_score, coding_score, extraction_score, cost_tier, total_calls, success_calls,
                task_fit_notes, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                provider_id = excluded.provider_id,
                model_name = excluded.model_name,
                context_window = excluded.context_window,
                supports_tools = excluded.supports_tools,
                supports_vision = excluded.supports_vision,
                supports_thinking = excluded.supports_thinking,
                supported_thinking_levels = excluded.supported_thinking_levels,
                default_thinking_level = excluded.default_thinking_level,
                latency_ms = excluded.latency_ms,
                reasoning_score = excluded.reasoning_score,
                coding_score = excluded.coding_score,
                extraction_score = excluded.extraction_score,
                cost_tier = excluded.cost_tier,
                total_calls = excluded.total_calls,
                success_calls = excluded.success_calls,
                task_fit_notes = excluded.task_fit_notes,
                updated_at = excluded.updated_at",
        )
        .bind(&profile.id)
        .bind(&profile.provider_id)
        .bind(&profile.model_name)
        .bind(profile.context_window)
        .bind(bool_int(profile.supports_tools))
        .bind(bool_int(profile.supports_vision))
        .bind(bool_int(profile.supports_thinking))
        .bind(ThinkingLevel::encode_list(&profile.supported_thinking_levels))
        .bind(profile.default_thinking_level.as_str())
        .bind(profile.latency_ms)
        .bind(profile.reasoning_score)
        .bind(profile.coding_score)
        .bind(profile.extraction_score)
        .bind(profile.cost_tier.0)
        .bind(profile.total_calls)
        .bind(profile.success_calls)
        .bind(&profile.task_fit_notes)
        .bind(time::to_db(profile.updated_at))
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Persist just the user-tunable knobs exposed in the capability matrix.
    pub async fn update_model_settings(
        &self,
        model_id: &str,
        default_thinking_level: &str,
        cost_tier: i64,
    ) -> StoreResult<ModelProfile> {
        let level = ThinkingLevel::parse(default_thinking_level)
            .map_err(|err| StoreError::invalid(err.to_string()))?;
        let tier = CostTier::new(cost_tier);

        let existing = self
            .get_model_profile(model_id)
            .await?
            .ok_or_else(|| StoreError::not_found(format!("model profile {model_id}")))?;

        let mut supported = existing.supported_thinking_levels.clone();
        if !supported.contains(&level) {
            supported.push(level);
            supported.sort();
            supported.dedup();
        }

        sqlx::query(
            "UPDATE model_profiles
             SET default_thinking_level = ?, cost_tier = ?, supported_thinking_levels = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(level.as_str())
        .bind(tier.0)
        .bind(ThinkingLevel::encode_list(&supported))
        .bind(time::now_db())
        .bind(model_id)
        .execute(self.pool())
        .await?;

        Ok(ModelProfile {
            default_thinking_level: level,
            cost_tier: tier,
            supported_thinking_levels: supported,
            updated_at: time::now(),
            ..existing
        })
    }

    /// Fold one empirical observation into the model's rolling statistics.
    pub async fn record_model_outcome(&self, model_id: &str, success: bool, latency_ms: i64) -> StoreResult<()> {
        let Some(mut profile) = self.get_model_profile(model_id).await? else {
            return Ok(());
        };
        profile.record_outcome(success);
        if latency_ms > 0 {
            profile.observe_latency(latency_ms);
        }
        sqlx::query(
            "UPDATE model_profiles
             SET total_calls = ?, success_calls = ?, latency_ms = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(profile.total_calls)
        .bind(profile.success_calls)
        .bind(profile.latency_ms)
        .bind(time::now_db())
        .bind(model_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn delete_model_profile(&self, model_id: &str) -> StoreResult<()> {
        sqlx::query("DELETE FROM model_profiles WHERE id = ?").bind(model_id).execute(self.pool()).await?;
        Ok(())
    }

    pub async fn count_models(&self) -> StoreResult<i64> {
        let row = sqlx::query("SELECT COUNT(*) AS c FROM model_profiles").fetch_one(self.pool()).await?;
        Ok(i(&row, "c"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::domain::{Provider, Protocol};

    async fn seeded() -> (Store, String) {
        let store = Store::connect_in_memory().await.unwrap();
        let provider = Provider::create("P", Protocol::OpenAiCompat, "https://api.example.com/v1", "sk-abc-123");
        store.upsert_provider(&provider).await.unwrap();
        (store, provider.id)
    }

    #[tokio::test]
    async fn profile_roundtrip() {
        let (store, provider_id) = seeded().await;
        let mut profile = ModelProfile::provisional(&provider_id, "gpt-4o");
        profile.supports_vision = true;
        profile.supports_thinking = true;
        profile.supported_thinking_levels = ThinkingLevel::all().to_vec();
        profile.default_thinking_level = ThinkingLevel::High;
        store.upsert_model_profile(&profile).await.unwrap();

        let loaded = store.get_model_profile(&profile.id).await.unwrap().unwrap();
        assert!(loaded.supports_vision);
        assert_eq!(loaded.default_thinking_level, ThinkingLevel::High);
        assert_eq!(loaded.supported_thinking_levels.len(), 5);
    }

    #[tokio::test]
    async fn provider_delete_cascades_models() {
        let (store, provider_id) = seeded().await;
        store.upsert_model_profile(&ModelProfile::provisional(&provider_id, "m1")).await.unwrap();
        assert_eq!(store.count_models().await.unwrap(), 1);
        store.delete_provider(&provider_id).await.unwrap();
        assert_eq!(store.count_models().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn settings_update_extends_supported_levels() {
        let (store, provider_id) = seeded().await;
        let profile = ModelProfile::provisional(&provider_id, "m");
        store.upsert_model_profile(&profile).await.unwrap();

        let updated = store.update_model_settings(&profile.id, "high", 3).await.unwrap();
        assert_eq!(updated.default_thinking_level, ThinkingLevel::High);
        assert_eq!(updated.cost_tier.0, 3);
        assert!(updated.supported_thinking_levels.contains(&ThinkingLevel::High));
    }

    #[tokio::test]
    async fn invalid_thinking_level_is_rejected() {
        let (store, provider_id) = seeded().await;
        let profile = ModelProfile::provisional(&provider_id, "m");
        store.upsert_model_profile(&profile).await.unwrap();
        assert!(store.update_model_settings(&profile.id, "galactic", 2).await.is_err());
        assert!(store.update_model_settings("missing", "high", 2).await.is_err());
    }

    #[tokio::test]
    async fn outcomes_accumulate() {
        let (store, provider_id) = seeded().await;
        let mut profile = ModelProfile::provisional(&provider_id, "m");
        profile.latency_ms = 1000;
        store.upsert_model_profile(&profile).await.unwrap();

        store.record_model_outcome(&profile.id, true, 2000).await.unwrap();
        store.record_model_outcome(&profile.id, false, 0).await.unwrap();

        let loaded = store.get_model_profile(&profile.id).await.unwrap().unwrap();
        assert_eq!(loaded.total_calls, 2);
        assert_eq!(loaded.success_calls, 1);
        assert_eq!(loaded.latency_ms, 1300);
    }
}
