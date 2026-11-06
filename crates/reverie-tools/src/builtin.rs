//! Assembles the global registry: twelve native primitives plus the meta tools
//! the orchestrator needs.

use std::sync::Arc;

use crate::attachment_tool::InspectAttachmentTool;
use crate::blackboard_tools::blackboard_tools;
use crate::forge::{load_verified_tools, ForgeDynamicToolTool};
use crate::fs_tools::fs_tools;
use crate::http::HttpApiRequestTool;
use crate::meta_tools::meta_tools;
use crate::registry::{AgentTool, GlobalToolRegistry};
use crate::shell::ShellExecuteTool;
use crate::web::web_tools;

/// Names of the twelve built-in native primitives, in documentation order.
pub const NATIVE_TOOL_NAMES: [&str; 12] = [
    "web_search",
    "web_fetch_clean",
    "inspect_attachment",
    "fs_read_file",
    "fs_write_file",
    "fs_edit_patch",
    "fs_list_and_search",
    "shell_execute",
    "http_api_request",
    "blackboard_publish",
    "blackboard_query",
    "forge_dynamic_tool",
];

/// Names of the meta-agent tools.
pub const META_TOOL_NAMES: [&str; 4] = [
    "configure_session_strategy",
    "spawn_sub_agents_batch",
    "finalize_task",
    "inspect_swarm",
];

/// Build the registry with only the built-in tools.
pub fn build_registry() -> Arc<GlobalToolRegistry> {
    let registry = Arc::new(GlobalToolRegistry::new());

    let mut tools: Vec<Arc<dyn AgentTool>> = Vec::new();
    tools.extend(web_tools());
    tools.push(Arc::new(InspectAttachmentTool));
    tools.extend(fs_tools());
    tools.push(Arc::new(ShellExecuteTool));
    tools.push(Arc::new(HttpApiRequestTool));
    tools.extend(blackboard_tools());
    tools.push(Arc::new(ForgeDynamicToolTool));
    tools.extend(meta_tools());

    for tool in tools {
        registry.register(tool);
    }
    registry
}

/// Build the registry and hot-register every previously verified forged tool.
pub async fn build_registry_with_dynamic(store: &reverie_store::Store) -> Arc<GlobalToolRegistry> {
    let registry = build_registry();
    for tool in load_verified_tools(store).await {
        registry.register(tool);
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_native_tool_is_registered() {
        let registry = build_registry();
        for name in NATIVE_TOOL_NAMES {
            assert!(registry.contains(name), "missing native tool {name}");
        }
        for name in META_TOOL_NAMES {
            assert!(registry.contains(name), "missing meta tool {name}");
        }
        assert_eq!(registry.len(), NATIVE_TOOL_NAMES.len() + META_TOOL_NAMES.len());
    }

    #[test]
    fn specs_are_complete() {
        let registry = build_registry();
        for spec in registry.specs(None) {
            assert!(!spec.name.is_empty());
            assert!(spec.description.len() > 20, "{} has a thin description", spec.name);
            assert_eq!(spec.parameters["type"], "object");
        }
    }

    #[tokio::test]
    async fn dynamic_tools_are_loaded_from_store() {
        let store = reverie_store::Store::connect_in_memory().await.unwrap();
        let registry = build_registry_with_dynamic(&store).await;
        assert_eq!(registry.len(), NATIVE_TOOL_NAMES.len() + META_TOOL_NAMES.len());
    }
}
