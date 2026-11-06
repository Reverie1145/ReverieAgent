//! Meta-agent tools: the levers the Meta-Orchestrator pulls to shape the swarm.

use async_trait::async_trait;
use reverie_core::domain::{AgentArchetype, AgentSpec, TaskCategory, ToolOutcome};
use reverie_core::llm::ThinkingLevel;
use serde_json::{json, Value};

use crate::args::{opt_str, opt_string_array, req_str};
use crate::context::ToolExecutionContext;
use crate::error::{ToolError, ToolResult};
use crate::registry::{AgentTool, ToolCategory};

/// Hard ceiling on how many agents a single batch may spawn.
const MAX_BATCH: usize = 500;

fn bridge(ctx: &ToolExecutionContext) -> ToolResult<&std::sync::Arc<dyn crate::context::OrchestrationBridge>> {
    ctx.orchestration
        .as_ref()
        .ok_or_else(|| ToolError::unavailable("this session has no swarm scheduler attached"))
}

/// Choose the topology, context strategy and isolation mode.
pub struct ConfigureStrategyTool;

#[async_trait]
impl AgentTool for ConfigureStrategyTool {
    fn name(&self) -> &str {
        "configure_session_strategy"
    }

    fn description(&self) -> &str {
        "Decide how the swarm is wired before spawning agents: the topology (flat_swarm, \
         hierarchical_tree, dag_pipeline), the context routing strategy (blackboard, pub_sub, \
         peer_to_peer, hybrid) and the workspace isolation mode (shared_vfs_lock, git_worktree, \
         sandbox_dir)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "topology": {
                    "type": "string",
                    "enum": ["flat_swarm", "hierarchical_tree", "dag_pipeline"]
                },
                "context_strategy": {
                    "type": "string",
                    "enum": ["blackboard", "pub_sub", "peer_to_peer", "hybrid"]
                },
                "isolation_strategy": {
                    "type": "string",
                    "enum": ["shared_vfs_lock", "git_worktree", "sandbox_dir"]
                },
                "rationale": {"type": "string", "description": "One sentence on why this wiring fits the task"}
            },
            "required": ["topology", "context_strategy", "isolation_strategy", "rationale"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let topology = req_str(&args, "topology")?;
        let context_strategy = req_str(&args, "context_strategy")?;
        let isolation = req_str(&args, "isolation_strategy")?;
        let rationale = req_str(&args, "rationale")?;

        let confirmation = bridge(ctx)?
            .configure_strategy(&ctx.session_id, &topology, &context_strategy, &isolation)
            .await?;
        ctx.emit_activity(format!("Strategy: {topology} / {context_strategy} / {isolation}"));

        Ok(ToolOutcome::ok_with(
            format!("{confirmation}\nRationale recorded: {rationale}"),
            json!({
                "topology": topology,
                "context_strategy": context_strategy,
                "isolation_strategy": isolation
            }),
        ))
    }
}

/// Spawn a batch of sub-agents.
pub struct SpawnSubAgentsTool;

fn parse_archetype(raw: &str) -> ToolResult<AgentArchetype> {
    AgentArchetype::parse(raw).map_err(|err| ToolError::invalid(err.to_string()))
}

fn parse_thinking(raw: Option<String>) -> ToolResult<Option<ThinkingLevel>> {
    match raw {
        None => Ok(None),
        Some(value) => ThinkingLevel::parse_optional(&value).map_err(|err| ToolError::invalid(err.to_string())),
    }
}

#[async_trait]
impl AgentTool for SpawnSubAgentsTool {
    fn name(&self) -> &str {
        "spawn_sub_agents_batch"
    }

    fn description(&self) -> &str {
        "Spawn a batch of sub-agents (1-500). Each agent gets a role, an archetype, a task, an \
         optional model and thinking depth, optional attachments, an optional tool allow-list, and \
         optional dependencies on other agents' role names (which creates DAG edges)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "agents": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_BATCH,
                    "items": {
                        "type": "object",
                        "properties": {
                            "role_name": {"type": "string", "description": "Unique, descriptive, e.g. Auth-Module-Dev"},
                            "archetype": {
                                "type": "string",
                                "enum": [
                                    "researcher", "coder", "reviewer", "analyst", "writer",
                                    "vision_inspector", "data_extractor", "merge_resolver", "generalist"
                                ]
                            },
                            "task_description": {"type": "string"},
                            "model_id": {"type": "string", "description": "Optional explicit model; omit to auto-route"},
                            "thinking_level": {
                                "type": "string",
                                "enum": ["off", "low", "medium", "high", "max"]
                            },
                            "depends_on_roles": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Role names of already-spawned agents this one waits for"
                            },
                            "attached_file_ids": {"type": "array", "items": {"type": "string"}},
                            "allowed_tools": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Restrict this agent to these tools; omit for all tools"
                            },
                            "task_category": {
                                "type": "string",
                                "enum": [
                                    "orchestration", "architecture", "coding", "debugging", "research",
                                    "extraction", "vision_inspection", "summarization", "general"
                                ]
                            }
                        },
                        "required": ["role_name", "archetype", "task_description"]
                    }
                }
            },
            "required": ["agents"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let raw_agents = args
            .get("agents")
            .and_then(Value::as_array)
            .ok_or_else(|| ToolError::invalid("`agents` must be an array"))?;
        if raw_agents.is_empty() {
            return Err(ToolError::invalid("spawn at least one agent"));
        }
        if raw_agents.len() > MAX_BATCH {
            return Err(ToolError::invalid(format!("a single batch is limited to {MAX_BATCH} agents")));
        }

        // Resolve role-name dependencies against already-persisted agents.
        let existing = ctx.store.list_agents(&ctx.session_id).await?;
        let mut specs: Vec<AgentSpec> = Vec::with_capacity(raw_agents.len());
        let mut seen_roles: Vec<String> = Vec::new();

        for entry in raw_agents {
            let role_name = req_str(entry, "role_name")?;
            if seen_roles.contains(&role_name) {
                return Err(ToolError::invalid(format!("duplicate role_name `{role_name}` in batch")));
            }
            let archetype = parse_archetype(&req_str(entry, "archetype")?)?;
            let task_description = req_str(entry, "task_description")?;

            let mut spec = AgentSpec::new(role_name.clone(), archetype, task_description);
            spec.model_id = opt_str(entry, "model_id");
            spec.thinking_level = parse_thinking(opt_str(entry, "thinking_level"))?;
            spec.attached_file_ids = opt_string_array(entry, "attached_file_ids");
            let allowed = opt_string_array(entry, "allowed_tools");
            if !allowed.is_empty() {
                spec.allowed_tools = Some(allowed);
            }
            if let Some(category) = opt_str(entry, "task_category") {
                spec.task_category =
                    Some(TaskCategory::parse(&category).map_err(|err| ToolError::invalid(err.to_string()))?);
            }

            for dependency in opt_string_array(entry, "depends_on_roles") {
                match existing.iter().find(|run| run.role_name == dependency) {
                    Some(run) => spec.depend_on_agent_ids.push(run.id.clone()),
                    None => {
                        return Err(ToolError::invalid(format!(
                            "`{role_name}` depends on `{dependency}`, which has not been spawned yet. \
                             Spawn dependencies in an earlier batch."
                        )))
                    }
                }
            }

            spec.validate()?;
            seen_roles.push(role_name);
            specs.push(spec);
        }

        let count = specs.len();
        let ids = bridge(ctx)?.spawn_sub_agents(&ctx.session_id, &ctx.agent_id, specs).await?;
        ctx.emit_activity(format!("Spawned {count} sub-agent(s)"));

        Ok(ToolOutcome::ok_with(
            format!("Spawned {count} agent(s):\n{}", ids.join("\n")),
            json!({"spawned": count, "agent_ids": ids}),
        ))
    }
}

/// Close out the session with the final deliverable.
pub struct FinalizeTaskTool;

#[async_trait]
impl AgentTool for FinalizeTaskTool {
    fn name(&self) -> &str {
        "finalize_task"
    }

    fn description(&self) -> &str {
        "Deliver the final synthesised answer for the whole session. Call this exactly once, when \
         every sub-agent has finished and their findings have been merged. The summary is rendered \
         as the deliverable in the UI."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Complete final deliverable in Markdown, with sections, tables and citations"
                }
            },
            "required": ["summary"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
        let summary = req_str(&args, "summary")?;
        if summary.trim().is_empty() {
            return Err(ToolError::invalid("summary must not be empty"));
        }
        bridge(ctx)?.finalize_session(&ctx.session_id, &ctx.agent_id, &summary).await?;
        ctx.emit_activity("Delivered the final report");
        Ok(ToolOutcome::ok("Session finalized. The deliverable has been recorded."))
    }
}

/// Inspect the current swarm state.
pub struct InspectSwarmTool;

#[async_trait]
impl AgentTool for InspectSwarmTool {
    fn name(&self) -> &str {
        "inspect_swarm"
    }

    fn description(&self) -> &str {
        "Read the live swarm snapshot: every agent's role, status, model and thinking depth, plus \
         how many tokens it has consumed. Use this before deciding whether to spawn more agents or \
         to finalise."
    }

    fn parameters(&self) -> Value {
        json!({"type": "object", "properties": {}, "required": []})
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn execute(&self, ctx: &ToolExecutionContext, _args: Value) -> ToolResult<ToolOutcome> {
        let snapshot = bridge(ctx)?.swarm_status(&ctx.session_id).await?;
        Ok(ToolOutcome::ok(ctx.cap_output(snapshot)))
    }
}

/// All four meta tools.
pub fn meta_tools() -> Vec<std::sync::Arc<dyn AgentTool>> {
    vec![
        std::sync::Arc::new(ConfigureStrategyTool),
        std::sync::Arc::new(SpawnSubAgentsTool),
        std::sync::Arc::new(FinalizeTaskTool),
        std::sync::Arc::new(InspectSwarmTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thinking_parsing_accepts_auto() {
        assert_eq!(parse_thinking(Some("auto".into())).unwrap(), None);
        assert_eq!(parse_thinking(Some("high".into())).unwrap(), Some(ThinkingLevel::High));
        assert!(parse_thinking(Some("galactic".into())).is_err());
    }

    #[test]
    fn archetype_parsing_rejects_unknown() {
        assert_eq!(parse_archetype("coder").unwrap(), AgentArchetype::Coder);
        assert!(parse_archetype("wizard").is_err());
    }

    #[test]
    fn spawn_schema_documents_limits() {
        let params = SpawnSubAgentsTool.parameters();
        assert_eq!(params["properties"]["agents"]["maxItems"], MAX_BATCH);
        assert_eq!(params["required"][0], "agents");
    }

    #[test]
    fn tool_names_are_stable() {
        let names: Vec<String> = meta_tools().iter().map(|tool| tool.name().to_string()).collect();
        assert_eq!(
            names,
            vec![
                "configure_session_strategy",
                "spawn_sub_agents_batch",
                "finalize_task",
                "inspect_swarm"
            ]
        );
    }
}
