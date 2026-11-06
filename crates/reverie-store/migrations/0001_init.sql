-- ReverieAgent initial schema.
-- All timestamps are RFC 3339 UTC strings (TEXT) so ordering is lexicographic
-- and the values stay readable in any SQLite browser.

CREATE TABLE IF NOT EXISTS providers (
    id                          TEXT PRIMARY KEY,
    name                        TEXT NOT NULL,
    protocol                    TEXT NOT NULL CHECK (protocol IN ('openai_compat', 'anthropic', 'gemini', 'ollama')),
    base_url                    TEXT NOT NULL,
    api_key                     TEXT NOT NULL,
    search_engine_config_json   TEXT NOT NULL DEFAULT '{}',
    is_enabled                  INTEGER NOT NULL DEFAULT 1,
    created_at                  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_profiles (
    id                          TEXT PRIMARY KEY,
    provider_id                 TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    model_name                  TEXT NOT NULL,
    context_window              INTEGER NOT NULL DEFAULT 32768,
    supports_tools              INTEGER NOT NULL DEFAULT 1,
    supports_vision             INTEGER NOT NULL DEFAULT 0,
    supports_thinking           INTEGER NOT NULL DEFAULT 0,
    supported_thinking_levels   TEXT NOT NULL DEFAULT '["off"]',
    default_thinking_level      TEXT NOT NULL DEFAULT 'medium',
    latency_ms                  INTEGER NOT NULL DEFAULT 1000,
    reasoning_score             REAL NOT NULL DEFAULT 5.0,
    coding_score                REAL NOT NULL DEFAULT 5.0,
    extraction_score            REAL NOT NULL DEFAULT 5.0,
    cost_tier                   INTEGER NOT NULL DEFAULT 2,
    total_calls                 INTEGER NOT NULL DEFAULT 0,
    success_calls               INTEGER NOT NULL DEFAULT 0,
    task_fit_notes              TEXT NOT NULL DEFAULT '',
    updated_at                  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_model_profiles_provider ON model_profiles(provider_id);

CREATE TABLE IF NOT EXISTS sessions (
    id                      TEXT PRIMARY KEY,
    title                   TEXT NOT NULL,
    workspace_path          TEXT NOT NULL,
    user_objective          TEXT NOT NULL,
    thinking_mode_override  TEXT NOT NULL DEFAULT 'auto',
    web_search_enabled      INTEGER NOT NULL DEFAULT 1,
    status                  TEXT NOT NULL CHECK (status IN ('planning', 'running', 'paused', 'completed', 'failed')),
    topology_mode           TEXT NOT NULL CHECK (topology_mode IN ('flat_swarm', 'hierarchical_tree', 'dag_pipeline')),
    context_strategy        TEXT NOT NULL CHECK (context_strategy IN ('blackboard', 'pub_sub', 'peer_to_peer', 'hybrid')),
    isolation_strategy      TEXT NOT NULL CHECK (isolation_strategy IN ('shared_vfs_lock', 'git_worktree', 'sandbox_dir')),
    final_summary           TEXT,
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_updated ON sessions(updated_at DESC);

CREATE TABLE IF NOT EXISTS session_attachments (
    id                      TEXT PRIMARY KEY,
    session_id              TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    file_name               TEXT NOT NULL,
    file_path               TEXT NOT NULL,
    mime_type               TEXT NOT NULL,
    file_size_bytes         INTEGER NOT NULL,
    base64_thumbnail        TEXT,
    extracted_text_preview  TEXT,
    created_at              TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_attachments_session ON session_attachments(session_id);

CREATE TABLE IF NOT EXISTS agent_runs (
    id                      TEXT PRIMARY KEY,
    session_id              TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    parent_agent_id         TEXT,
    role_name               TEXT NOT NULL,
    archetype               TEXT NOT NULL,
    model_id                TEXT NOT NULL,
    thinking_level          TEXT NOT NULL DEFAULT 'medium',
    task_description        TEXT NOT NULL,
    depend_on_agent_ids     TEXT NOT NULL DEFAULT '[]',
    attached_file_ids       TEXT NOT NULL DEFAULT '[]',
    workspace_dir           TEXT NOT NULL,
    git_branch              TEXT,
    status                  TEXT NOT NULL CHECK (status IN ('queued', 'running', 'waiting_dependency', 'completed', 'failed', 'cancelled')),
    message_history_json    TEXT NOT NULL DEFAULT '[]',
    output_artifact         TEXT,
    prompt_tokens           INTEGER NOT NULL DEFAULT 0,
    completion_tokens       INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens        INTEGER NOT NULL DEFAULT 0,
    error_message           TEXT,
    created_at              TEXT NOT NULL,
    finished_at             TEXT
);

CREATE INDEX IF NOT EXISTS idx_agent_runs_session ON agent_runs(session_id);
CREATE INDEX IF NOT EXISTS idx_agent_runs_status ON agent_runs(session_id, status);

CREATE TABLE IF NOT EXISTS context_blackboard (
    id                  TEXT PRIMARY KEY,
    session_id          TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    source_agent_id     TEXT NOT NULL,
    topic_tag           TEXT NOT NULL,
    summary             TEXT NOT NULL,
    full_content        TEXT NOT NULL,
    target_agent_ids    TEXT NOT NULL DEFAULT '["*"]',
    created_at          TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_blackboard_session ON context_blackboard(session_id, created_at DESC);

CREATE TABLE IF NOT EXISTS dynamic_tools (
    id                      TEXT PRIMARY KEY,
    tool_name               TEXT NOT NULL UNIQUE,
    description             TEXT NOT NULL,
    parameters_json_schema  TEXT NOT NULL,
    interpreter             TEXT NOT NULL CHECK (interpreter IN ('python3', 'node', 'bash')),
    script_path             TEXT NOT NULL,
    created_by_agent_id     TEXT,
    is_verified             INTEGER NOT NULL DEFAULT 1,
    usage_count             INTEGER NOT NULL DEFAULT 0,
    created_at              TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS evolution_store (
    id          TEXT PRIMARY KEY,
    record_type TEXT NOT NULL CHECK (record_type IN ('archetype_prompt', 'playbook_skill')),
    key_name    TEXT NOT NULL UNIQUE,
    title       TEXT NOT NULL,
    content     TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1,
    changelog   TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
