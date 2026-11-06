# ReverieAgent

An autonomous, massively parallel multi-agent orchestration desktop application.
Rust owns the primitives — actor runtime, virtual file locks, git worktree
provisioning, context blackboard, live web search, multimodal file pipelines,
script sandboxes. The Meta-Agents own the policy: they decide the topology, who
works on what, which model each agent uses, and how deep it thinks.

```
┌───────────────────────────── Tauri v2 shell ─────────────────────────────┐
│  React 18 + TS + Tailwind        ◀── reverie-event ──┐                   │
│  Tasks · Models · Toolbox · Evolution                │                   │
└──────────────────────┬───────────────────────────────┼───────────────────┘
                       │ IPC commands                  │ events
┌──────────────────────▼───────────────────────────────┴───────────────────┐
│ reverie-desktop   command surface + event bridge                        │
├──────────────────────────────────────────────────────────────────────────┤
│ reverie-runtime   SwarmScheduler · MetaOrchestrator · ContextBroker      │
│                   EnvironmentWarden · EvolutionEngine                    │
├──────────────────────────────────────────────────────────────────────────┤
│ reverie-llm       4 protocol adapters · thinking-depth translation       │
│                   model profiler · dynamic router                        │
├──────────────────────────────────────────────────────────────────────────┤
│ reverie-tools     12 native primitives · VFS locks · tool forge          │
├──────────────────────────────────────────────────────────────────────────┤
│ reverie-store     SQLite (WAL) repositories · 8 tables · migrations      │
├──────────────────────────────────────────────────────────────────────────┤
│ reverie-core      domain entities · LLM types · event taxonomy           │
└──────────────────────────────────────────────────────────────────────────┘
```

## Layout

```
crates/
  reverie-core/       domain value objects, thinking levels, event taxonomy
  reverie-store/      sqlx repositories + migrations/0001_init.sql
  reverie-llm/        adapters/{openai,anthropic,gemini,ollama}, profiler/, router
  reverie-tools/      web/, fs_tools, shell, http, blackboard, forge, meta_tools
  reverie-runtime/    scheduler, agent_actor, context_broker, warden, evolution
  reverie-desktop/    Tauri v2 app + commands/
apps/desktop/         React + TypeScript + Tailwind UI
scripts/make-icons.cjs  dependency-free PNG/ICO generator for the app icons
```

No source file exceeds 1,000 lines; the largest is ~720.

## Running it

**Windows, one click:** double-click `start.bat` (or `start.bat release`). It checks the
toolchain, installs frontend dependencies on first run, builds the UI and the desktop
shell, then launches the app detached.

Manual:

```bash
# 1. frontend deps
npm --prefix apps/desktop install

# 2. desktop dev (starts Vite on :1420 and the Tauri window)
cargo run -p reverie-desktop

# production bundle
npm --prefix apps/desktop run build
cargo build -p reverie-desktop --release
```

`start.bat` is deliberately ASCII-only: `cmd.exe` decodes batch files with the active
OEM code page, and non-ASCII text breaks parsing on machines with a different one.

The SQLite database lives in the platform app-data directory as
`reverie_agent.db`; WAL is enabled so the swarm can write concurrently.

## First run

1. **Models & Search → Add provider.** Pick a protocol, paste a base URL and key.
   Ollama needs no key.
2. **Pull model list** — the live roster is fetched from the provider.
3. **Run auto-profiler** — each model is probed concurrently for latency, tool
   support, vision support and thinking depth, then an evaluation agent assigns
   reasoning / coding / extraction scores, a cost tier and task-fit notes.
4. Optionally add a Tavily / Brave / Serper key or a SearXNG endpoint. With none
   configured, `web_search` scrapes DuckDuckGo and Bing with zero setup.
5. Pick a workspace folder, describe an objective, press **Run Task**.

## How a task executes

1. A **Meta-Orchestrator** agent starts with the objective and every mounted
   attachment. Its tools are `configure_session_strategy`,
   `spawn_sub_agents_batch`, `inspect_swarm` and `finalize_task`.
2. It chooses a topology (`flat_swarm`, `hierarchical_tree`, `dag_pipeline`), a
   context strategy (`blackboard`, `pub_sub`, `peer_to_peer`, `hybrid`) and an
   isolation mode (`shared_vfs_lock`, `git_worktree`, `sandbox_dir`).
3. Each spawned agent is routed to a model and a thinking depth. Planning and
   hard debugging get `high`/`max` on a frontier model; 200 parallel extraction
   workers get `off` on a cheap one. Both can be changed live from the inspector.
4. The **Context Broker** builds a budgeted briefing per agent: objective,
   mounted files, upstream deliverables, the blackboard slice addressed to it.
5. Tool-calling rounds are **unbounded** — an agent keeps working until it stops
   calling tools or the run is cancelled. A failed model response (transport,
   429, 5xx) is retried **5 times** with exponential backoff (1s → 16s); the
   partial stream is reset so the retry is not duplicated.
6. The **Environment Warden** provisions each workspace, and for `git_worktree`
   creates a branch per coding agent. On completion the branch is merged back; a
   conflict spawns a `Merge-Resolver` agent that reads both sides.
7. Every write emits a unified diff; every agent streams text and reasoning
   tokens separately.
8. When the orchestrator calls `finalize_task`, it waits for stragglers, then
   publishes the deliverable and closes the session.
9. The **Evolution Engine** folds the outcome back in: model scores are
   recalibrated from real telemetry, the strategy is distilled into a playbook,
   and role prompts absorb lessons from failures.

## Thinking-depth translation

| Level  | OpenAI `reasoning_effort` | Anthropic `budget_tokens` | Gemini `thinkingBudget` | Ollama `think` |
| ------ | ------------------------- | ------------------------- | ----------------------- | -------------- |
| off    | omitted                   | block omitted             | 0                       | false          |
| low    | `low`                     | 2 048                     | 1 024                   | true           |
| medium | `medium`                  | 8 192                     | 8 192                   | true           |
| high   | `high`                    | 16 384                    | 24 576                  | true           |
| max    | `high`                    | 32 000                    | -1 (dynamic)            | true           |

Reasoning tokens are parsed separately from the visible answer in every
protocol (`reasoning_content`, `thinking_delta`, `thought: true`, `thinking`),
and inline `<think>…</think>` blocks from local models are split out in real
time.

## Native tool primitives

`web_search` · `web_fetch_clean` · `inspect_attachment` · `fs_read_file` ·
`fs_write_file` · `fs_edit_patch` · `fs_list_and_search` · `shell_execute` ·
`http_api_request` · `blackboard_publish` · `blackboard_query` ·
`forge_dynamic_tool`

Plus the meta tools: `configure_session_strategy`, `spawn_sub_agents_batch`,
`finalize_task`, `inspect_swarm`.

`forge_dynamic_tool` writes a python3/node/bash script into
`.reverie/custom_tools/`, runs a 15-second sandbox verification with the supplied
test input, and only then persists and hot-registers it for the whole swarm.

## Database

Eight tables: `providers`, `model_profiles`, `sessions`, `session_attachments`,
`agent_runs`, `context_blackboard`, `dynamic_tools`, `evolution_store`.
See `crates/reverie-store/migrations/0001_init.sql`.

## Verification

```bash
cargo check --workspace --all-targets   # 0 errors, 0 warnings
cargo test  --workspace                 # 300 tests
npm --prefix apps/desktop run build     # tsc --noEmit && vite build
```

The desktop binary has been booted end to end: on first launch it creates
`reverie_agent.db` with `journal_mode=wal`, all 8 tables and 12 seeded archetype
prompts, then opens its window.
