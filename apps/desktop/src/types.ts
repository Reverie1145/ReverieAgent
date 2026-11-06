/**
 * TypeScript mirror of the Rust wire format.
 *
 * Every Rust struct serializes with snake_case field names, and every enum
 * serializes its variant as a lowercase string (or an internally tagged
 * `kind`/`type` discriminant), so these types map one-to-one.
 */

export type ThinkingLevel = 'off' | 'low' | 'medium' | 'high' | 'max';
export type ThinkingMode = 'auto' | ThinkingLevel;

export const THINKING_LEVELS: ThinkingLevel[] = ['off', 'low', 'medium', 'high', 'max'];
export const THINKING_MODES: ThinkingMode[] = ['auto', 'off', 'low', 'medium', 'high', 'max'];

export type Protocol = 'openai_compat' | 'anthropic' | 'gemini' | 'ollama';

export type SessionStatus = 'planning' | 'running' | 'paused' | 'completed' | 'failed';
export type AgentStatus =
  | 'queued'
  | 'running'
  | 'waiting_dependency'
  | 'completed'
  | 'failed'
  | 'cancelled';
export type TopologyMode = 'flat_swarm' | 'hierarchical_tree' | 'dag_pipeline';
export type ContextStrategy = 'blackboard' | 'pub_sub' | 'peer_to_peer' | 'hybrid';
export type IsolationStrategy = 'shared_vfs_lock' | 'git_worktree' | 'sandbox_dir';

export interface SearchEngineConfig {
  tavily_api_key?: string;
  brave_api_key?: string;
  serper_api_key?: string;
  searxng_endpoint?: string;
  preferred_engine?: string;
}

export interface ProviderView {
  id: string;
  name: string;
  protocol: Protocol;
  base_url: string;
  api_key_masked: string;
  has_api_key: boolean;
  is_enabled: boolean;
  search_engine_config: SearchEngineConfig;
  model_count: number;
  created_at: string;
}

export interface ProviderInput {
  id?: string;
  name: string;
  protocol: Protocol;
  base_url: string;
  api_key?: string;
  is_enabled?: boolean;
  search_engine_config?: SearchEngineConfig;
}

export interface ModelProfile {
  id: string;
  provider_id: string;
  model_name: string;
  context_window: number;
  supports_tools: boolean;
  supports_vision: boolean;
  supports_thinking: boolean;
  supported_thinking_levels: ThinkingLevel[];
  default_thinking_level: ThinkingLevel;
  latency_ms: number;
  reasoning_score: number;
  coding_score: number;
  extraction_score: number;
  cost_tier: number;
  total_calls: number;
  success_calls: number;
  task_fit_notes: string;
  updated_at: string;
}

export interface Session {
  id: string;
  title: string;
  workspace_path: string;
  user_objective: string;
  thinking_mode_override: string;
  web_search_enabled: boolean;
  status: SessionStatus;
  topology_mode: TopologyMode;
  context_strategy: ContextStrategy;
  isolation_strategy: IsolationStrategy;
  final_summary: string | null;
  created_at: string;
  updated_at: string;
}

export interface SessionAttachment {
  id: string;
  session_id: string;
  file_name: string;
  file_path: string;
  mime_type: string;
  file_size_bytes: number;
  base64_thumbnail: string | null;
  extracted_text_preview: string | null;
  created_at: string;
}

export interface AgentSummary {
  id: string;
  session_id: string;
  parent_agent_id: string | null;
  role_name: string;
  archetype: string;
  archetype_label: string;
  model_id: string;
  model_name: string;
  thinking_level: ThinkingLevel;
  status: AgentStatus;
  task_description: string;
  workspace_dir: string;
  git_branch: string | null;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  elapsed_ms: number;
  activity: string;
  error_message: string | null;
  depends_on: string[];
  attached_file_ids: string[];
  output_preview: string | null;
}

export interface BlackboardEntry {
  id: string;
  session_id: string;
  source_agent_id: string;
  topic_tag: string;
  summary: string;
  full_content: string;
  target_agent_ids: string[];
  created_at: string;
}

export interface TokenTotals {
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
}

export interface SessionSnapshot {
  session: Session;
  attachments: SessionAttachment[];
  agents: AgentSummary[];
  blackboard: BlackboardEntry[];
  final_summary: string | null;
  totals: TokenTotals;
  active: boolean;
}

export type ContentPart =
  | { type: 'text'; text: string }
  | { type: 'reasoning'; text: string }
  | { type: 'image_base64'; mime_type: string; data: string }
  | { type: 'file_document'; file_name: string; mime_type: string; content: string };

export interface ToolCall {
  id: string;
  name: string;
  arguments: unknown;
}

export interface ChatMessage {
  role: 'system' | 'user' | 'assistant' | 'tool';
  parts: ContentPart[];
  tool_calls: ToolCall[];
  tool_call_id?: string;
  name?: string;
}

export interface AgentDetail {
  agent: AgentSummary;
  messages: ChatMessage[];
  attachments: SessionAttachment[];
  published: BlackboardEntry[];
}

export interface ToolDescriptor {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  is_dynamic: boolean;
  interpreter: string | null;
  usage_count: number;
  created_by_agent_id: string | null;
}

export interface EvolutionRecord {
  id: string;
  record_type: 'archetype_prompt' | 'playbook_skill';
  key_name: string;
  title: string;
  content: string;
  version: number;
  changelog: string;
  updated_at: string;
}

export interface AppInfo {
  name: string;
  version: string;
  db_path: string;
  default_workspace: string;
  tool_count: number;
  model_count: number;
  session_count: number;
  provider_count: number;
}

export interface StageResult {
  attachments: SessionAttachment[];
  errors: string[];
}

export interface EvolutionReport {
  session_id: string;
  models_updated: number;
  playbooks_written: number;
  prompts_evolved: number;
  notes: string[];
}

/* ------------------------------------------------------------------ */
/* Live events                                                         */
/* ------------------------------------------------------------------ */

export type SystemEvent =
  | { kind: 'agent_stream_chunk'; session_id: string; agent_id: string; delta_text: string; is_reasoning: boolean }
  | { kind: 'agent_stream_reset'; session_id: string; agent_id: string }
  | { kind: 'agent_activity'; session_id: string; agent_id: string; activity: string }
  | {
      kind: 'agent_created';
      session_id: string;
      agent_id: string;
      role_name: string;
      archetype: string;
      model_id: string;
      thinking_level: string;
      status: string;
    }
  | { kind: 'agent_status_changed'; session_id: string; agent_id: string; status: string; error_message: string | null }
  | { kind: 'agent_route_changed'; session_id: string; agent_id: string; model_id: string; thinking_level: string }
  | {
      kind: 'agent_token_usage';
      session_id: string;
      agent_id: string;
      prompt_tokens: number;
      completion_tokens: number;
      reasoning_tokens: number;
    }
  | {
      kind: 'agent_tool_call';
      session_id: string;
      agent_id: string;
      call_id: string;
      tool_name: string;
      arguments: unknown;
    }
  | {
      kind: 'agent_tool_result';
      session_id: string;
      agent_id: string;
      call_id: string;
      tool_name: string;
      is_error: boolean;
      preview: string;
    }
  | { kind: 'session_status_changed'; session_id: string; status: string }
  | {
      kind: 'session_strategy_changed';
      session_id: string;
      topology: string;
      context_strategy: string;
      isolation: string;
    }
  | {
      kind: 'blackboard_published';
      session_id: string;
      entry_id: string;
      topic_tag: string;
      source_agent_id: string;
      summary: string;
    }
  | { kind: 'workspace_diff_updated'; session_id: string; agent_id: string; file_path: string; diff: string }
  | { kind: 'workspace_file_changed'; session_id: string; paths: string[] }
  | { kind: 'search_progress'; session_id: string; agent_id: string; query: string; engine: string; results_found: number }
  | { kind: 'profiling_progress'; provider_id: string; stage: string; message: string; completed: number; total: number }
  | { kind: 'model_profile_updated'; model_id: string }
  | { kind: 'session_completed'; session_id: string; summary: string }
  | { kind: 'log_message'; level: string; scope: string; message: string };

/* ------------------------------------------------------------------ */
/* UI-only models                                                      */
/* ------------------------------------------------------------------ */

export interface LiveToolCall {
  callId: string;
  toolName: string;
  args: unknown;
  isError?: boolean;
  preview?: string;
}

export interface AgentLiveState {
  activity: string;
  thinking: string;
  output: string;
  toolCalls: LiveToolCall[];
  promptTokens: number;
  completionTokens: number;
  reasoningTokens: number;
}

export interface DiffEntry {
  agentId: string;
  filePath: string;
  diff: string;
  receivedAt: number;
}

export interface Toast {
  id: string;
  level: 'info' | 'success' | 'error';
  message: string;
}

export type ViewKey = 'tasks' | 'models' | 'tools' | 'evolution';
