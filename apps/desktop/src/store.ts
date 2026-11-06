import { create } from 'zustand';
import { api, isDesktop } from './api';
import { translate, type Lang } from './i18n';
import type {
  AgentDetail,
  AgentLiveState,
  AgentSummary,
  AppInfo,
  DiffEntry,
  EvolutionRecord,
  ModelProfile,
  ProviderInput,
  ProviderView,
  SearchEngineConfig,
  Session,
  SessionAttachment,
  SessionSnapshot,
  SystemEvent,
  ThinkingMode,
  Toast,
  ToolDescriptor,
  ViewKey,
} from './types';

const EMPTY_LIVE: AgentLiveState = {
  activity: '',
  thinking: '',
  output: '',
  toolCalls: [],
  promptTokens: 0,
  completionTokens: 0,
  reasoningTokens: 0,
};

let snapshotTimer: ReturnType<typeof setTimeout> | null = null;
let modelsTimer: ReturnType<typeof setTimeout> | null = null;

interface AppStore {
  ready: boolean;
  bridgeAvailable: boolean;
  view: ViewKey;
  theme: 'light' | 'dark';
  lang: Lang;

  providers: ProviderView[];
  models: ModelProfile[];
  sessions: Session[];
  tools: ToolDescriptor[];
  evolution: EvolutionRecord[];
  info: AppInfo | null;

  activeSessionId: string | null;
  snapshot: SessionSnapshot | null;
  live: Record<string, AgentLiveState>;
  diffs: DiffEntry[];
  externalChanges: { count: number; paths: string[] };

  inspectorAgentId: string | null;
  inspector: AgentDetail | null;

  workspacePath: string;
  attachments: SessionAttachment[];
  webSearch: boolean;
  thinkingMode: ThinkingMode;
  selectedModelId: string;

  busy: Record<string, boolean>;
  profiling: { active: boolean; message: string; completed: number; total: number };
  toasts: Toast[];

  init: () => Promise<void>;
  setView: (view: ViewKey) => void;
  toggleTheme: () => void;
  setLang: (lang: Lang) => void;
  toggleLang: () => void;
  setWorkspace: (path: string) => void;
  setWebSearch: (value: boolean) => void;
  setThinkingMode: (value: ThinkingMode) => void;
  setSelectedModel: (value: string) => void;

  refreshProviders: () => Promise<void>;
  refreshModels: () => Promise<void>;
  refreshSessions: () => Promise<void>;
  refreshTools: () => Promise<void>;
  refreshEvolution: () => Promise<void>;
  refreshInfo: () => Promise<void>;

  saveProvider: (payload: ProviderInput) => Promise<boolean>;
  removeProvider: (id: string) => Promise<void>;
  saveSearchConfig: (providerId: string, config: SearchEngineConfig) => Promise<void>;
  pullModels: (providerId: string) => Promise<void>;
  profileModels: (providerId?: string) => Promise<void>;
  updateModelSettings: (modelId: string, level: string, tier: number) => Promise<void>;

  selectSession: (sessionId: string) => Promise<void>;
  refreshSnapshot: () => Promise<void>;
  startTask: (objective: string) => Promise<boolean>;
  controlSession: (action: 'pause' | 'resume' | 'cancel') => Promise<void>;
  deleteSession: (sessionId: string) => Promise<void>;
  reflectSession: (sessionId: string) => Promise<void>;

  openInspector: (agentId: string) => Promise<void>;
  closeInspector: () => void;
  refreshInspector: () => Promise<void>;
  rerouteAgent: (agentId: string, modelId: string | null, thinking: string | null) => Promise<void>;

  addAttachments: (paths: string[]) => Promise<void>;
  addAttachmentBytes: (fileName: string, mimeType: string, base64: string) => Promise<void>;
  removeAttachment: (id: string) => void;
  clearExternalChanges: () => void;

  saveEvolution: (id: string, content: string) => Promise<void>;

  pushToast: (level: Toast['level'], message: string) => void;
  dismissToast: (id: string) => void;

  handleEvent: (event: SystemEvent) => void;
}

function scheduleSnapshotRefresh(get: () => AppStore) {
  if (snapshotTimer) clearTimeout(snapshotTimer);
  snapshotTimer = setTimeout(() => {
    snapshotTimer = null;
    void get().refreshSnapshot();
  }, 400);
}

function scheduleModelRefresh(get: () => AppStore) {
  if (modelsTimer) clearTimeout(modelsTimer);
  modelsTimer = setTimeout(() => {
    modelsTimer = null;
    void get().refreshModels();
  }, 600);
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Stored preference wins; otherwise follow the OS locale. */
function initialLang(): Lang {
  try {
    const stored = localStorage.getItem('reverie.lang');
    if (stored === 'zh' || stored === 'en') return stored;
  } catch {
    /* storage may be unavailable */
  }
  const locale = typeof navigator !== 'undefined' ? navigator.language.toLowerCase() : 'en';
  return locale.startsWith('zh') ? 'zh' : 'en';
}

export const useApp = create<AppStore>()((set, get) => ({
  ready: false,
  bridgeAvailable: isDesktop,
  view: 'tasks',
  theme: 'light',
  lang: initialLang(),

  providers: [],
  models: [],
  sessions: [],
  tools: [],
  evolution: [],
  info: null,

  activeSessionId: null,
  snapshot: null,
  live: {},
  diffs: [],
  externalChanges: { count: 0, paths: [] },

  inspectorAgentId: null,
  inspector: null,

  workspacePath: '',
  attachments: [],
  webSearch: true,
  thinkingMode: 'auto',
  selectedModelId: '',

  busy: {},
  profiling: { active: false, message: '', completed: 0, total: 0 },
  toasts: [],

  async init() {
    if (!isDesktop) {
      set({ ready: true, bridgeAvailable: false });
      get().pushToast('error', translate(get().lang, 'toast.bridgeMissing'));
      return;
    }

    // Render the shell first: a slow or failing backend must never leave the
    // window stuck on the splash screen.
    set({ ready: true });

    try {
      const info = await api.appInfo();
      set({ info, workspacePath: get().workspacePath || info.default_workspace });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }

    await Promise.all([
      get().refreshProviders(),
      get().refreshModels(),
      get().refreshSessions(),
      get().refreshTools(),
      get().refreshEvolution(),
    ]);
  },

  setView(view) {
    set({ view });
  },

  toggleTheme() {
    const next = get().theme === 'light' ? 'dark' : 'light';
    set({ theme: next });
    document.documentElement.classList.toggle('dark', next === 'dark');
  },

  setLang(lang) {
    set({ lang });
    try {
      localStorage.setItem('reverie.lang', lang);
    } catch {
      /* storage may be unavailable */
    }
  },

  toggleLang() {
    get().setLang(get().lang === 'zh' ? 'en' : 'zh');
  },

  setWorkspace(path) {
    set({ workspacePath: path });
  },

  setWebSearch(value) {
    set({ webSearch: value });
  },

  setThinkingMode(value) {
    set({ thinkingMode: value });
  },

  setSelectedModel(value) {
    set({ selectedModelId: value });
  },

  async refreshProviders() {
    try {
      set({ providers: await api.listProviders() });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async refreshModels() {
    try {
      set({ models: await api.listModelProfiles() });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async refreshSessions() {
    try {
      set({ sessions: await api.listSessions() });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async refreshTools() {
    try {
      set({ tools: await api.listTools() });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async refreshEvolution() {
    try {
      set({ evolution: await api.listEvolution() });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async refreshInfo() {
    try {
      set({ info: await api.appInfo() });
    } catch {
      /* non-critical */
    }
  },

  async saveProvider(payload) {
    try {
      const saved = await api.upsertProvider(payload);
      await get().refreshProviders();
      get().pushToast('success', translate(get().lang, 'toast.providerSaved', { name: saved.name }));
      return true;
    } catch (error) {
      get().pushToast('error', errorMessage(error));
      return false;
    }
  },

  async removeProvider(id) {
    try {
      await api.deleteProvider(id);
      await Promise.all([get().refreshProviders(), get().refreshModels()]);
      get().pushToast('success', translate(get().lang, 'toast.providerRemoved'));
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async saveSearchConfig(providerId, config) {
    try {
      await api.updateSearchConfig(providerId, config);
      await get().refreshProviders();
      get().pushToast('success', translate(get().lang, 'toast.searchSaved'));
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async pullModels(providerId) {
    set({ busy: { ...get().busy, [providerId]: true } });
    try {
      const models = await api.pullProviderModels(providerId);
      await get().refreshModels();
      await get().refreshProviders();
      get().pushToast('success', translate(get().lang, 'toast.modelsPulled', { count: models.length }));
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    } finally {
      set({ busy: { ...get().busy, [providerId]: false } });
    }
  },

  async profileModels(providerId) {
    set({ profiling: { active: true, message: 'Starting the profiler…', completed: 0, total: 0 } });
    try {
      const models = await api.profileProviderModels(providerId);
      await get().refreshModels();
      get().pushToast('success', translate(get().lang, 'toast.modelsProfiled', { count: models.length }));
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    } finally {
      set({ profiling: { active: false, message: '', completed: 0, total: 0 } });
    }
  },

  async updateModelSettings(modelId, level, tier) {
    try {
      await api.updateModelSettings(modelId, level, tier);
      await get().refreshModels();
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async selectSession(sessionId) {
    set({
      activeSessionId: sessionId,
      view: 'tasks',
      snapshot: null,
      inspectorAgentId: null,
      inspector: null,
      externalChanges: { count: 0, paths: [] },
    });
    await get().refreshSnapshot();
  },

  async refreshSnapshot() {
    const sessionId = get().activeSessionId;
    if (!sessionId) return;
    try {
      const snapshot = await api.sessionSnapshot(sessionId);
      set({ snapshot });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async startTask(objective) {
    const { workspacePath, attachments, thinkingMode, webSearch, selectedModelId } = get();
    if (!objective.trim()) {
      get().pushToast('error', translate(get().lang, 'toast.needObjective'));
      return false;
    }
    if (!workspacePath.trim()) {
      get().pushToast('error', translate(get().lang, 'toast.needWorkspace'));
      return false;
    }
    try {
      const session = await api.createSession({
        title: '',
        workspacePath,
        userObjective: objective.trim(),
        attachmentIds: attachments.map((item) => item.id),
        thinkingModeOverride: thinkingMode,
        webSearchEnabled: webSearch,
        orchestratorModelId: selectedModelId || null,
      });
      set({
        attachments: [],
        live: {},
        diffs: [],
        externalChanges: { count: 0, paths: [] },
        activeSessionId: session.id,
      });
      await get().refreshSessions();
      await get().refreshSnapshot();
      get().pushToast('success', translate(get().lang, 'toast.taskStarted'));
      return true;
    } catch (error) {
      get().pushToast('error', errorMessage(error));
      return false;
    }
  },

  async controlSession(action) {
    const sessionId = get().activeSessionId;
    if (!sessionId) return;
    try {
      await api.controlSession(sessionId, action);
      await get().refreshSnapshot();
      await get().refreshSessions();
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async deleteSession(sessionId) {
    try {
      await api.deleteSession(sessionId);
      if (get().activeSessionId === sessionId) {
        set({ activeSessionId: null, snapshot: null, inspectorAgentId: null, inspector: null });
      }
      await get().refreshSessions();
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async reflectSession(sessionId) {
    try {
      const report = await api.reflectSession(sessionId);
      await Promise.all([get().refreshEvolution(), get().refreshModels()]);
      get().pushToast(
        'success',
        translate(get().lang, 'toast.reflection', {
          models: report.models_updated,
          playbooks: report.playbooks_written,
          prompts: report.prompts_evolved,
        }),
      );
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async openInspector(agentId) {
    set({ inspectorAgentId: agentId, inspector: null });
    await get().refreshInspector();
  },

  closeInspector() {
    set({ inspectorAgentId: null, inspector: null });
  },

  async refreshInspector() {
    const agentId = get().inspectorAgentId;
    if (!agentId) return;
    try {
      set({ inspector: await api.agentDetail(agentId) });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async rerouteAgent(agentId, modelId, thinking) {
    try {
      await api.updateAgentRoute(agentId, modelId, thinking);
      await get().refreshInspector();
      await get().refreshSnapshot();
      get().pushToast('success', translate(get().lang, 'toast.rerouted'));
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async addAttachments(paths) {
    const workspace = get().workspacePath;
    if (!paths.length) return;
    if (!workspace.trim()) {
      get().pushToast('error', translate(get().lang, 'toast.attachFirst'));
      return;
    }
    try {
      const result = await api.stageAttachments(paths, workspace);
      set({ attachments: [...get().attachments, ...result.attachments] });
      if (result.errors.length) {
        get().pushToast('error', result.errors.join(' · '));
      }
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  async addAttachmentBytes(fileName, mimeType, base64) {
    const workspace = get().workspacePath;
    if (!workspace.trim()) {
      get().pushToast('error', translate(get().lang, 'toast.attachFirst'));
      return;
    }
    try {
      const attachment = await api.stageAttachmentBytes({
        fileName,
        mimeType,
        base64Data: base64,
        workspacePath: workspace,
      });
      set({ attachments: [...get().attachments, attachment] });
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  removeAttachment(id) {
    set({ attachments: get().attachments.filter((item) => item.id !== id) });
  },

  clearExternalChanges() {
    set({ externalChanges: { count: 0, paths: [] } });
  },
  async saveEvolution(id, content) {
    try {
      await api.saveEvolution(id, content);
      await get().refreshEvolution();
      get().pushToast('success', translate(get().lang, 'toast.recordSaved'));
    } catch (error) {
      get().pushToast('error', errorMessage(error));
    }
  },

  pushToast(level, message) {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    set({ toasts: [...get().toasts, { id, level, message }] });
    setTimeout(() => get().dismissToast(id), 5200);
  },

  dismissToast(id) {
    set({ toasts: get().toasts.filter((toast) => toast.id !== id) });
  },

  handleEvent(event) {
    switch (event.kind) {
      case 'agent_stream_chunk': {
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        const next: AgentLiveState = event.is_reasoning
          ? { ...current, thinking: current.thinking + event.delta_text }
          : { ...current, output: current.output + event.delta_text };
        set({ live: { ...get().live, [event.agent_id]: next } });
        break;
      }
      case 'agent_stream_reset': {
        // A failed turn is being retried: drop the partial streamed text so the
        // retry does not appear duplicated. Tool calls are kept as history.
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        set({
          live: {
            ...get().live,
            [event.agent_id]: { ...current, thinking: '', output: '' },
          },
        });
        break;
      }
      case 'agent_activity': {
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        set({ live: { ...get().live, [event.agent_id]: { ...current, activity: event.activity } } });
        break;
      }
      case 'agent_token_usage': {
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        set({
          live: {
            ...get().live,
            [event.agent_id]: {
              ...current,
              promptTokens: event.prompt_tokens,
              completionTokens: event.completion_tokens,
              reasoningTokens: event.reasoning_tokens,
            },
          },
        });
        break;
      }
      case 'agent_tool_call': {
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        set({
          live: {
            ...get().live,
            [event.agent_id]: {
              ...current,
              toolCalls: [
                ...current.toolCalls,
                { callId: event.call_id, toolName: event.tool_name, args: event.arguments },
              ],
            },
          },
        });
        break;
      }
      case 'agent_tool_result': {
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        set({
          live: {
            ...get().live,
            [event.agent_id]: {
              ...current,
              toolCalls: current.toolCalls.map((call) =>
                call.callId === event.call_id
                  ? { ...call, isError: event.is_error, preview: event.preview }
                  : call,
              ),
            },
          },
        });
        break;
      }
      case 'agent_created':
      case 'agent_status_changed':
      case 'agent_route_changed':
        scheduleSnapshotRefresh(get);
        if (get().inspectorAgentId === event.agent_id) {
          void get().refreshInspector();
        }
        break;
      case 'session_status_changed': {
        const snapshot = get().snapshot;
        if (snapshot && snapshot.session.id === event.session_id) {
          set({ snapshot: { ...snapshot, session: { ...snapshot.session, status: event.status as Session['status'] } } });
        }
        void get().refreshSessions();
        break;
      }
      case 'session_strategy_changed': {
        const snapshot = get().snapshot;
        if (snapshot && snapshot.session.id === event.session_id) {
          set({
            snapshot: {
              ...snapshot,
              session: {
                ...snapshot.session,
                topology_mode: event.topology as Session['topology_mode'],
                context_strategy: event.context_strategy as Session['context_strategy'],
                isolation_strategy: event.isolation as Session['isolation_strategy'],
              },
            },
          });
        }
        break;
      }
      case 'blackboard_published':
        scheduleSnapshotRefresh(get);
        break;
      case 'workspace_diff_updated': {
        const entry: DiffEntry = {
          agentId: event.agent_id,
          filePath: event.file_path,
          diff: event.diff,
          receivedAt: Date.now(),
        };
        set({ diffs: [...get().diffs.slice(-199), entry] });
        break;
      }
      case 'workspace_file_changed': {
        const previous = get().externalChanges;
        const merged = [...new Set([...previous.paths, ...event.paths])].slice(-30);
        set({ externalChanges: { count: previous.count + event.paths.length, paths: merged } });
        scheduleSnapshotRefresh(get);
        break;
      }
      case 'search_progress': {
        const current = get().live[event.agent_id] ?? EMPTY_LIVE;
        set({
          live: {
            ...get().live,
            [event.agent_id]: {
              ...current,
              activity: `Searched ${event.engine} for “${event.query}” → ${event.results_found} result(s)`,
            },
          },
        });
        break;
      }
      case 'profiling_progress':
        set({
          profiling: {
            active: event.stage !== 'done' && event.stage !== 'error',
            message: event.message,
            completed: event.completed,
            total: event.total,
          },
        });
        break;
      case 'model_profile_updated':
        scheduleModelRefresh(get);
        break;
      case 'session_completed': {
        const snapshot = get().snapshot;
        if (snapshot && snapshot.session.id === event.session_id) {
          set({ snapshot: { ...snapshot, final_summary: event.summary } });
        }
        scheduleSnapshotRefresh(get);
        break;
      }
      case 'log_message':
        if (event.level === 'error') {
          get().pushToast('error', `${event.scope}: ${event.message}`);
        }
        break;
      default:
        break;
    }
  },
}));

/** Agents that are still working, for the grid filter. */
export function isActiveAgent(agent: AgentSummary): boolean {
  return agent.status === 'running' || agent.status === 'queued' || agent.status === 'waiting_dependency';
}
