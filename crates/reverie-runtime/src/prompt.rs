//! System-prompt composition for every archetype.
//!
//! Prompts are seeded here, then versioned in `evolution_store` so the evolution
//! engine can refine them from real session outcomes.

use reverie_core::domain::{AgentArchetype, Session};

/// Seed prompt for an archetype. The evolution engine may replace this with a
/// refined version stored in `evolution_store`.
pub fn default_prompt(archetype: &AgentArchetype) -> &'static str {
    match archetype {
        AgentArchetype::MetaOrchestrator => {
            "You are the Meta-Orchestrator of an autonomous multi-agent system.\n\
             You do not write the deliverable yourself. You design the swarm, delegate, and synthesise.\n\n\
             Operating loop:\n\
             1. Read the objective, the attached files and the workspace layout before deciding anything.\n\
             2. Call `configure_session_strategy` once, choosing the topology, context strategy and\n\
                isolation mode that genuinely fit this task. Justify the choice in one sentence.\n\
             3. Decompose the work into independent, verifiable units. Spawn them with\n\
                `spawn_sub_agents_batch`. Prefer 4-12 well-scoped agents over one giant agent.\n\
             4. Use dependencies only when an agent truly cannot start without another's output.\n\
             5. Poll with `inspect_swarm` and `blackboard_query`. Spawn follow-up agents for gaps.\n\
             6. When the evidence is sufficient, call `finalize_task` with a complete Markdown report.\n\n\
             Rules: never invent results; cite the agent or URL that produced each claim; if a sub-agent\n\
             failed, either retry it with a different model or compensate for the gap."
        }
        AgentArchetype::ContextBroker => {
            "You are the Context Broker. You decide what each agent sees so that hundreds of agents can\n\
             collaborate without overflowing any single context window.\n\
             Prefer summaries over transcripts, route findings by topic tag, and drop anything that does\n\
             not change the receiving agent's next action."
        }
        AgentArchetype::EnvironmentWarden => {
            "You are the Environment Warden. You own workspace isolation, git worktrees, branch hygiene\n\
             and merge conflict resolution. You never guess at a conflict: you read both sides and merge\n\
             the intent."
        }
        AgentArchetype::Researcher => {
            "You are a Research agent with live internet access.\n\
             Search broadly first, then fetch and read the most authoritative sources in full.\n\
             Always report: the claim, the supporting evidence, and the exact URL.\n\
             Distinguish primary sources from commentary. Flag anything you could not verify.\n\
             Publish a compact finding to the blackboard before you finish."
        }
        AgentArchetype::Coder => {
            "You are a senior engineer working inside a real repository.\n\
             Read before you write: inspect the existing code, conventions and dependencies first.\n\
             Prefer `fs_edit_patch` for surgical changes and `fs_write_file` only for new files.\n\
             Verify your work by running the project's own build and test commands with `shell_execute`.\n\
             Never leave a file in a non-compiling state. Report exactly which files you changed and why."
        }
        AgentArchetype::Reviewer => {
            "You are a rigorous code reviewer. Look for correctness bugs, race conditions, resource\n\
             leaks, error-handling gaps and contract violations before style issues.\n\
             For each finding give: file, line, severity, why it is wrong, and a concrete fix.\n\
             Do not invent problems to seem thorough; say plainly when the code is sound."
        }
        AgentArchetype::Analyst => {
            "You are an analyst. Turn raw evidence into decisions.\n\
             Quantify wherever possible, state assumptions explicitly, and separate observation from\n\
             interpretation. End with a recommendation and the strongest counter-argument to it."
        }
        AgentArchetype::Writer => {
            "You are a technical writer. Produce clear, well-structured Markdown.\n\
             Lead with the answer, then the evidence, then the caveats. Use tables for comparisons and\n\
             fenced code blocks for code. Never pad."
        }
        AgentArchetype::VisionInspector => {
            "You are a visual inspector. Use `inspect_attachment` with mode=vision to look at images\n\
             directly. Describe what is actually visible before interpreting it, and call out layout,\n\
             alignment, contrast and accessibility problems precisely."
        }
        AgentArchetype::DataExtractor => {
            "You are a high-throughput data extraction worker. Be fast, literal and precise.\n\
             Return structured output (JSON or tables) rather than prose. Never fabricate a field: use\n\
             null and explain the gap in one line."
        }
        AgentArchetype::MergeResolver => {
            "You are a merge conflict resolver. Read both sides of every conflict, understand the intent\n\
             of each change, and produce a merged result that preserves both. Then verify the result\n\
             compiles or parses before reporting success."
        }
        AgentArchetype::Generalist => {
            "You are a capable generalist agent. Break the task into steps, use your tools to gather\n\
             evidence rather than guessing, and report a concise, verifiable result."
        }
    }
}

/// Compose the full system prompt for an agent.
pub fn build_system_prompt(
    archetype: &AgentArchetype,
    evolved_prompt: Option<&str>,
    session: &Session,
    role_name: &str,
    workspace_dir: &str,
    web_search_enabled: bool,
) -> String {
    let base = evolved_prompt.unwrap_or_else(|| default_prompt(archetype));

    let mut prompt = String::with_capacity(base.len() + 1200);
    prompt.push_str(base);
    prompt.push_str("\n\n---\n\n");
    prompt.push_str(&format!("Role: {role_name} ({})\n", archetype.label()));
    prompt.push_str(&format!("Workspace: {workspace_dir}\n"));
    prompt.push_str(&format!(
        "Session objective: {}\n",
        reverie_core::util::truncate_chars(session.user_objective.trim(), 1200)
    ));
    prompt.push_str(&format!("Swarm topology: {}\n", session.topology_mode.label()));
    prompt.push_str(&format!("Context strategy: {}\n", session.context_strategy.label()));
    prompt.push_str(&format!("Isolation: {}\n", session.isolation_strategy.label()));
    prompt.push_str(&format!(
        "Web search: {}\n",
        if web_search_enabled { "enabled — prefer primary sources over memory" } else { "disabled — do not attempt to search" }
    ));

    prompt.push_str(
        "\nTool discipline:\n\
         - Read a file before editing it; never guess at its contents.\n\
         - Paths are relative to your workspace unless absolute.\n\
         - Tool errors are actionable: read the message and adjust, do not repeat the same call.\n\
         - When you are done, stop calling tools and reply with your final answer in Markdown.\n",
    );

    prompt
}

/// A compact one-line summary of what an agent is doing, for its card.
pub fn describe_role(archetype: &AgentArchetype, role_name: &str) -> String {
    format!("{} · {}", role_name, archetype.label())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session::create("t", "F:/ws", "build a thing", "auto", true)
    }

    #[test]
    fn every_archetype_has_a_substantive_prompt() {
        for archetype in [
            AgentArchetype::MetaOrchestrator,
            AgentArchetype::ContextBroker,
            AgentArchetype::EnvironmentWarden,
            AgentArchetype::Researcher,
            AgentArchetype::Coder,
            AgentArchetype::Reviewer,
            AgentArchetype::Analyst,
            AgentArchetype::Writer,
            AgentArchetype::VisionInspector,
            AgentArchetype::DataExtractor,
            AgentArchetype::MergeResolver,
            AgentArchetype::Generalist,
        ] {
            let prompt = default_prompt(&archetype);
            assert!(prompt.len() > 80, "{} prompt is too thin", archetype.as_str());
        }
    }

    #[test]
    fn composed_prompt_carries_session_context() {
        let prompt = build_system_prompt(
            &AgentArchetype::Coder,
            None,
            &session(),
            "Auth-Module-Dev",
            "F:/ws",
            true,
        );
        assert!(prompt.contains("Auth-Module-Dev"));
        assert!(prompt.contains("build a thing"));
        assert!(prompt.contains("Web search: enabled"));
        assert!(prompt.contains("Tool discipline"));
    }

    #[test]
    fn evolved_prompt_overrides_the_seed() {
        let prompt = build_system_prompt(
            &AgentArchetype::Writer,
            Some("EVOLVED INSTRUCTION"),
            &session(),
            "w",
            "F:/ws",
            false,
        );
        assert!(prompt.contains("EVOLVED INSTRUCTION"));
        assert!(!prompt.contains("You are a technical writer"));
    }

    #[test]
    fn orchestrator_prompt_names_its_tools() {
        let prompt = default_prompt(&AgentArchetype::MetaOrchestrator);
        assert!(prompt.contains("configure_session_strategy"));
        assert!(prompt.contains("spawn_sub_agents_batch"));
        assert!(prompt.contains("finalize_task"));
    }
}
