import { invoke } from '@tauri-apps/api/core';
import type {
  AgentDetail,
  AppInfo,
  EvolutionRecord,
  EvolutionReport,
  ModelProfile,
  ProviderInput,
  ProviderView,
  SearchEngineConfig,
  Session,
  SessionAttachment,
  SessionSnapshot,
  StageResult,
  ToolDescriptor,
} from './types';

/** True when running inside the Tauri webview. */
export const isDesktop =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in (window as unknown as Record<string, unknown>);

const BRIDGE_MISSING =
  'The desktop bridge is unavailable. Start ReverieAgent through Tauri (npm run tauri dev) instead of a plain browser.';

/**
 * Default ceiling for an IPC round-trip.
 *
 * A panic inside a Rust command handler aborts the task without ever resolving
 * the promise, which would leave the UI spinning forever. Every call therefore
 * races a timeout and surfaces a real error instead.
 */
const DEFAULT_TIMEOUT_MS = 60_000;

function withTimeout<T>(promise: Promise<T>, command: string, timeoutMs: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(
        new Error(
          `\`${command}\` did not respond within ${Math.round(timeoutMs / 1000)}s. ` +
            'The backend may have crashed — check the app console.',
        ),
      );
    }, timeoutMs);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}

async function call<T>(command: string, args?: Record<string, unknown>, timeoutMs = DEFAULT_TIMEOUT_MS): Promise<T> {
  if (!isDesktop) {
    throw new Error(BRIDGE_MISSING);
  }
  try {
    return await withTimeout(invoke<T>(command, args), command, timeoutMs);
  } catch (error) {
    const message = typeof error === 'string' ? error : error instanceof Error ? error.message : String(error);
    throw new Error(message);
  }
}

/** Long-running commands get a larger budget. */
const LONG_TIMEOUT_MS = 15 * 60_000;
const MEDIUM_TIMEOUT_MS = 3 * 60_000;

export const api = {
  appInfo: () => call<AppInfo>('cmd_get_app_info'),

  listProviders: () => call<ProviderView[]>('cmd_list_providers'),
  upsertProvider: (payload: ProviderInput) => call<ProviderView>('cmd_upsert_provider', { payload }),
  deleteProvider: (id: string) => call<void>('cmd_delete_provider', { id }),
  updateSearchConfig: (providerId: string, searchEngineConfig: SearchEngineConfig) =>
    call<ProviderView>('cmd_update_provider_search_config', { providerId, searchEngineConfig }),

  listModelProfiles: () => call<ModelProfile[]>('cmd_list_model_profiles'),
  pullProviderModels: (providerId: string) =>
    call<ModelProfile[]>('cmd_pull_provider_models', { providerId }, MEDIUM_TIMEOUT_MS),
  profileProviderModels: (providerId?: string) =>
    call<ModelProfile[]>('cmd_profile_provider_models', { providerId: providerId ?? null }, LONG_TIMEOUT_MS),
  updateModelSettings: (modelId: string, defaultThinkingLevel: string, costTier: number) =>
    call<ModelProfile>('cmd_update_model_profile_settings', {
      modelId,
      defaultThinkingLevel,
      costTier,
    }),

  listSessions: () => call<Session[]>('cmd_list_sessions'),
  sessionSnapshot: (sessionId: string) => call<SessionSnapshot>('cmd_get_session_snapshot', { sessionId }),
  createSession: (params: {
    title: string;
    workspacePath: string;
    userObjective: string;
    attachmentIds: string[];
    thinkingModeOverride: string;
    webSearchEnabled: boolean;
    orchestratorModelId?: string | null;
  }) => call<Session>('cmd_create_and_start_session', params),
  controlSession: (sessionId: string, action: 'pause' | 'resume' | 'cancel') =>
    call<string>('cmd_control_session', { sessionId, action }),
  deleteSession: (sessionId: string) => call<void>('cmd_delete_session', { sessionId }),
  reflectSession: (sessionId: string) =>
    call<EvolutionReport>('cmd_reflect_session', { sessionId }, MEDIUM_TIMEOUT_MS),

  agentDetail: (agentId: string) => call<AgentDetail>('cmd_get_agent_detail', { agentId }),
  updateAgentRoute: (agentId: string, newModelId: string | null, newThinkingLevel: string | null) =>
    call<void>('cmd_update_agent_thinking_or_model', { agentId, newModelId, newThinkingLevel }),

  stageAttachments: (filePaths: string[], workspacePath: string) =>
    call<StageResult>('cmd_stage_attachments', { filePaths, workspacePath }),
  stageAttachmentBytes: (params: {
    fileName: string;
    mimeType: string;
    base64Data: string;
    workspacePath: string;
  }) => call<SessionAttachment>('cmd_stage_attachment_bytes', params),

  listTools: () => call<ToolDescriptor[]>('cmd_list_all_tools'),

  listEvolution: () => call<EvolutionRecord[]>('cmd_list_evolution_records'),
  saveEvolution: (id: string, content: string) =>
    call<EvolutionRecord>('cmd_save_evolution_record', { id, content }),
};
