//! The global tool registry: built-in primitives plus runtime-forged tools.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::RwLock;
use reverie_core::domain::{ToolDescriptor, ToolOutcome};
use reverie_core::llm::ToolSpec;
use serde_json::Value;

use crate::context::ToolExecutionContext;
use crate::error::ToolResult;

/// Coarse grouping used by the Toolbox UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Web,
    Filesystem,
    Shell,
    Network,
    Context,
    Meta,
    Dynamic,
}

impl ToolCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolCategory::Web => "web",
            ToolCategory::Filesystem => "filesystem",
            ToolCategory::Shell => "shell",
            ToolCategory::Network => "network",
            ToolCategory::Context => "context",
            ToolCategory::Meta => "meta",
            ToolCategory::Dynamic => "dynamic",
        }
    }
}

/// A callable capability exposed to agents.
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> Value;
    fn category(&self) -> ToolCategory {
        ToolCategory::Filesystem
    }
    async fn execute(&self, ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome>;

    /// Whether this tool was forged at runtime by an agent.
    fn is_dynamic(&self) -> bool {
        false
    }

    fn interpreter(&self) -> Option<String> {
        None
    }

    fn usage_count(&self) -> i64 {
        0
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec::new(self.name(), self.description(), self.parameters())
    }

    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
            is_dynamic: self.is_dynamic(),
            interpreter: self.interpreter(),
            usage_count: self.usage_count(),
            created_by_agent_id: None,
        }
    }
}

/// Registry shared by every agent in the process.
#[derive(Default)]
pub struct GlobalToolRegistry {
    tools: RwLock<BTreeMap<String, Arc<dyn AgentTool>>>,
}

impl std::fmt::Debug for GlobalToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobalToolRegistry").field("count", &self.names().len()).finish()
    }
}

impl GlobalToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register or replace a tool by name.
    pub fn register(&self, tool: Arc<dyn AgentTool>) {
        self.tools.write().insert(tool.name().to_string(), tool);
    }

    /// Remove a tool, returning whether it existed.
    pub fn unregister(&self, name: &str) -> bool {
        self.tools.write().remove(name).is_some()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.tools.read().get(name).cloned()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.read().contains_key(name)
    }

    /// Sorted tool names.
    pub fn names(&self) -> Vec<String> {
        self.tools.read().keys().cloned().collect()
    }

    /// All specs, optionally restricted to an allow-list.
    pub fn specs(&self, allowed: Option<&[String]>) -> Vec<ToolSpec> {
        let guard = self.tools.read();
        guard
            .values()
            .filter(|tool| match allowed {
                Some(list) => list.iter().any(|name| name == tool.name()),
                None => true,
            })
            .map(|tool| tool.spec())
            .collect()
    }

    /// Descriptors for the Toolbox UI, built-ins first then forged tools.
    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        let mut out: Vec<ToolDescriptor> = self.tools.read().values().map(|tool| tool.descriptor()).collect();
        out.sort_by(|a, b| a.is_dynamic.cmp(&b.is_dynamic).then(a.name.cmp(&b.name)));
        out
    }

    /// Execute a tool, converting any failure into a model-readable error
    /// outcome rather than propagating it.
    pub async fn execute(&self, name: &str, ctx: &ToolExecutionContext, args: Value) -> ToolOutcome {
        let Some(tool) = self.get(name) else {
            return ToolOutcome::error(format!(
                "unknown tool `{name}`. Available tools: {}",
                self.names().join(", ")
            ));
        };
        match tool.execute(ctx, args).await {
            Ok(outcome) => outcome,
            Err(err) => ToolOutcome::error(err.to_string()),
        }
    }

    pub fn len(&self) -> usize {
        self.tools.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoTool;

    #[async_trait]
    impl AgentTool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo back the input"
        }

        fn parameters(&self) -> Value {
            serde_json::json!({"type": "object", "properties": {"text": {"type": "string"}}})
        }

        async fn execute(&self, _ctx: &ToolExecutionContext, args: Value) -> ToolResult<ToolOutcome> {
            let text = args.get("text").and_then(Value::as_str).unwrap_or("").to_string();
            Ok(ToolOutcome::ok(text))
        }
    }

    struct FailingTool;

    #[async_trait]
    impl AgentTool for FailingTool {
        fn name(&self) -> &str {
            "boom"
        }

        fn description(&self) -> &str {
            "Always fails"
        }

        fn parameters(&self) -> Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _ctx: &ToolExecutionContext, _args: Value) -> ToolResult<ToolOutcome> {
            Err(crate::error::ToolError::other("intentional failure"))
        }
    }

    #[test]
    fn registration_and_lookup() {
        let registry = GlobalToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        assert!(registry.contains("echo"));
        assert_eq!(registry.names(), vec!["echo".to_string()]);
        assert_eq!(registry.specs(None).len(), 1);
        assert!(registry.unregister("echo"));
        assert!(!registry.unregister("echo"));
    }

    #[test]
    fn allow_list_filters_specs() {
        let registry = GlobalToolRegistry::new();
        registry.register(Arc::new(EchoTool));
        registry.register(Arc::new(FailingTool));
        assert_eq!(registry.specs(None).len(), 2);
        assert_eq!(registry.specs(Some(&["echo".to_string()])).len(), 1);
        assert_eq!(registry.specs(Some(&["missing".to_string()])).len(), 0);
    }

    #[test]
    fn descriptors_put_builtins_first() {
        let registry = GlobalToolRegistry::new();
        registry.register(Arc::new(FailingTool));
        registry.register(Arc::new(EchoTool));
        let descriptors = registry.descriptors();
        assert_eq!(descriptors.len(), 2);
        assert!(!descriptors[0].is_dynamic);
    }
}
