//! Shared-context blackboard tools.

use async_trait::async_trait;
use reverie_core::domain::{BlackboardEntry, ToolOutcome};
use reverie_core::events::SystemEvent;
use serde_json::{json, Value};

use crate::args::{opt_i64_clamped, opt_str, opt_string_array, req_str};
use crate::context::ToolExecutionContext;
use crate::error::ToolResult;
use crate::registry::{AgentTool, ToolCategory};

/// Publish a structured finding to the session blackboard.
pub struct BlackboardPublishTool;

#[async_trait]
impl AgentTool for BlackboardPublishTool {
    fn name(&self) -> &str {
        "blackboard_publish"
    }

    fn description(&self) -> &str {
        "Publish a structured finding to the shared context blackboard so other agents can reuse it \
         without re-reading your full transcript. Keep `summary` short and self-contained."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topic_tag": {
                    "type": "string",
                    "description": "Short routing tag, e.g. `auth-design`, `api-contract`, `benchmark`"
                },
                "summary": {"type": "string", "description": "One or two sentences another agent can act on"},
                "full_content": {"type": "string", "description": "Complete finding, code, or data"},
                "target_agent_ids": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Restrict delivery to these agent ids; omit to broadcast"
                }
            },
            "required": ["topic_tag", "summary", "full_content"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Context
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let topic_tag = req_str(&args, "topic_tag")?;
        let summary = req_str(&args, "summary")?;
        let full_content = req_str(&args, "full_content")?;
        let targets = opt_string_array(&args, "target_agent_ids");

        let entry = BlackboardEntry::publish(
            &ctx.session_id,
            &ctx.agent_id,
            topic_tag.clone(),
            summary.clone(),
            full_content,
            targets,
        );
        ctx.store.insert_blackboard(&entry).await?;

        ctx.sink.emit(SystemEvent::BlackboardPublished {
            session_id: ctx.session_id.clone(),
            entry_id: entry.id.clone(),
            topic_tag: topic_tag.clone(),
            source_agent_id: ctx.agent_id.clone(),
            summary: summary.clone(),
        });
        ctx.emit_activity(format!("Published `{topic_tag}` to the blackboard"));

        Ok(ToolOutcome::ok_with(
            format!("Published `{topic_tag}` (id {}): {summary}", entry.id),
            json!({"entry_id": entry.id, "topic_tag": topic_tag}),
        ))
    }
}

/// Read findings back off the blackboard.
pub struct BlackboardQueryTool;

#[async_trait]
impl AgentTool for BlackboardQueryTool {
    fn name(&self) -> &str {
        "blackboard_query"
    }

    fn description(&self) -> &str {
        "Search the shared blackboard for findings published by other agents. Omit `query` to list \
         the most recent entries."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Free-text filter over tag, summary and content"},
                "limit": {"type": "integer", "description": "Maximum entries to return", "default": 20},
                "only_for_me": {
                    "type": "boolean",
                    "description": "Restrict to entries addressed to this agent",
                    "default": false
                }
            },
            "required": []
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Context
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let limit = opt_i64_clamped(&args, "limit", 20, 1, 200);
        let query = opt_str(&args, "query");
        let only_for_me = crate::args::opt_bool(&args, "only_for_me", false);

        let mut entries = match &query {
            Some(query) => ctx.store.query_blackboard(&ctx.session_id, query, limit).await?,
            None => ctx.store.list_blackboard(&ctx.session_id, limit).await?,
        };
        if only_for_me {
            entries.retain(|entry| entry.targets(&ctx.agent_id));
        }

        if entries.is_empty() {
            return Ok(ToolOutcome::ok("The blackboard has no matching entries yet."));
        }

        let mut output = format!("{} blackboard entr(ies)\n\n", entries.len());
        for entry in &entries {
            output.push_str(&entry.to_context_block());
            output.push_str("\n\n---\n\n");
        }

        Ok(ToolOutcome::ok_with(
            ctx.cap_output(output),
            json!({"count": entries.len()}),
        ))
    }
}

/// Both blackboard tools.
pub fn blackboard_tools() -> Vec<std::sync::Arc<dyn AgentTool>> {
    vec![std::sync::Arc::new(BlackboardPublishTool), std::sync::Arc::new(BlackboardQueryTool)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use reverie_core::events::NoopSink;
    use std::sync::Arc;

    async fn context() -> ToolExecutionContext {
        let store = Arc::new(reverie_store::Store::connect_in_memory().await.unwrap());
        let session = reverie_core::domain::Session::create("t", "F:/ws", "o", "auto", true);
        store.insert_session(&session).await.unwrap();
        ToolExecutionContext {
            session_id: session.id,
            agent_id: "agent-1".into(),
            role_name: "tester".into(),
            workspace_dir: std::env::temp_dir(),
            store,
            vfs: Arc::new(crate::vfs::VfsLockManager::new()),
            sink: Arc::new(NoopSink),
            http: reqwest::Client::new(),
            search_config: reverie_core::domain::SearchEngineConfig::default(),
            web_search_enabled: true,
            registry: Arc::new(crate::registry::GlobalToolRegistry::new()),
            orchestration: None,
            max_output_chars: 20_000,
        }
    }

    #[tokio::test]
    async fn publish_then_query() {
        let ctx = context().await;
        BlackboardPublishTool
            .execute(
                &ctx,
                json!({
                    "topic_tag": "auth",
                    "summary": "JWT with refresh rotation",
                    "full_content": "Details about JWT"
                }),
            )
            .await
            .unwrap();

        let outcome = BlackboardQueryTool.execute(&ctx, json!({"query": "jwt"})).await.unwrap();
        assert!(outcome.content.contains("JWT with refresh rotation"));
        assert_eq!(outcome.metadata["count"], 1);
    }

    #[tokio::test]
    async fn empty_query_returns_guidance() {
        let ctx = context().await;
        let outcome = BlackboardQueryTool.execute(&ctx, json!({})).await.unwrap();
        assert!(outcome.content.contains("no matching entries"));
    }

    #[tokio::test]
    async fn only_for_me_filters_targets() {
        let ctx = context().await;
        BlackboardPublishTool
            .execute(
                &ctx,
                json!({
                    "topic_tag": "t",
                    "summary": "s",
                    "full_content": "c",
                    "target_agent_ids": ["someone-else"]
                }),
            )
            .await
            .unwrap();
        let outcome = BlackboardQueryTool.execute(&ctx, json!({"only_for_me": true})).await.unwrap();
        assert!(outcome.content.contains("no matching entries"));
    }
}
