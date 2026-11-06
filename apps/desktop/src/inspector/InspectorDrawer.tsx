import { useEffect, useState } from 'react';
import { Activity, Files, Layers, RefreshCw, X } from 'lucide-react';
import { archetypeLabel, thinkLabel } from '../i18n';
import { useApp } from '../store';
import { useT } from '../useT';
import { THINKING_LEVELS } from '../types';
import {
  Badge,
  Button,
  IconButton,
  Select,
  StatusBadge,
  cx,
  formatDuration,
  formatTokens,
} from '../components/ui';
import { ActivityTab } from './ActivityTab';
import { ContextTab } from './ContextTab';
import { DiffTab } from './DiffTab';

type Tab = 'activity' | 'context' | 'files';

/** 460px slide-over inspector for a single sub-agent. */
export function InspectorDrawer() {
  const t = useT();
  const lang = useApp((state) => state.lang);
  const agentId = useApp((state) => state.inspectorAgentId);
  const detail = useApp((state) => state.inspector);
  const closeInspector = useApp((state) => state.closeInspector);
  const refreshInspector = useApp((state) => state.refreshInspector);
  const rerouteAgent = useApp((state) => state.rerouteAgent);
  const models = useApp((state) => state.models);

  const [tab, setTab] = useState<Tab>('activity');
  const [modelId, setModelId] = useState('');
  const [thinking, setThinking] = useState('medium');
  const [applying, setApplying] = useState(false);

  useEffect(() => {
    if (!detail) return;
    setModelId(detail.agent.model_id);
    setThinking(detail.agent.thinking_level);
  }, [detail]);

  useEffect(() => {
    setTab('activity');
  }, [agentId]);

  if (!agentId) return null;

  const agent = detail?.agent;
  const dirty = Boolean(agent && (modelId !== agent.model_id || thinking !== agent.thinking_level));

  const TABS: Array<{ key: Tab; label: string; icon: typeof Activity }> = [
    { key: 'activity', label: t('insp.activity'), icon: Activity },
    { key: 'context', label: t('insp.context'), icon: Layers },
    { key: 'files', label: t('insp.files'), icon: Files },
  ];

  async function apply() {
    if (!agent) return;
    setApplying(true);
    try {
      await rerouteAgent(
        agent.id,
        modelId !== agent.model_id ? modelId : null,
        thinking !== agent.thinking_level ? thinking : null,
      );
    } finally {
      setApplying(false);
    }
  }

  return (
    <aside className="fixed inset-y-0 right-0 z-40 flex w-[460px] flex-col border-l border-line bg-surface shadow-drawer animate-slide-in">
      <div className="flex items-start gap-2 border-b border-line px-3.5 py-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h2 className="truncate text-[13.5px] font-semibold text-ink">{agent?.role_name ?? 'Agent'}</h2>
            {agent ? <StatusBadge status={agent.status} /> : null}
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-1">
            <Badge tone="neutral">{agent ? archetypeLabel(lang, agent.archetype) : '—'}</Badge>
            {agent?.git_branch ? <Badge tone="accent">{agent.git_branch}</Badge> : null}
            {agent ? (
              <span className="text-[10.5px] text-muted">
                {formatDuration(agent.elapsed_ms)} ·{' '}
                {formatTokens(agent.prompt_tokens + agent.completion_tokens)} tok
              </span>
            ) : null}
          </div>
        </div>
        <IconButton label={t('common.refresh')} onClick={() => void refreshInspector()}>
          <RefreshCw size={13} />
        </IconButton>
        <IconButton label={t('common.close')} onClick={closeInspector}>
          <X size={14} />
        </IconButton>
      </div>

      <div className="border-b border-line px-3.5 py-2.5">
        <div className="flex items-end gap-2">
          <label className="min-w-0 flex-1">
            <span className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-muted">
              {t('insp.model')}
            </span>
            <Select
              value={modelId}
              onChange={(event) => setModelId(event.target.value)}
              className="h-7 w-full text-[11.5px]"
            >
              {agent && !models.some((model) => model.id === agent.model_id) ? (
                <option value={agent.model_id}>{agent.model_name || 'auto'}</option>
              ) : null}
              {models.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.model_name}
                </option>
              ))}
            </Select>
          </label>

          <label className="w-[104px]">
            <span className="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-muted">
              {t('insp.depth')}
            </span>
            <Select
              value={thinking}
              onChange={(event) => setThinking(event.target.value)}
              className="h-7 w-full text-[11.5px]"
            >
              {THINKING_LEVELS.map((level) => (
                <option key={level} value={level}>
                  {t(`think.${level}`)}
                </option>
              ))}
            </Select>
          </label>

          <Button
            size="sm"
            variant={dirty ? 'primary' : 'secondary'}
            disabled={!dirty || applying}
            onClick={() => void apply()}
          >
            {applying ? t('common.applying') : t('common.apply')}
          </Button>
        </div>
        <p className="mt-1 text-[10px] text-muted">{t('insp.applyHint')}</p>
      </div>

      <div className="flex items-center gap-1 border-b border-line px-2.5">
        {TABS.map((item) => {
          const Icon = item.icon;
          const active = tab === item.key;
          return (
            <button
              key={item.key}
              type="button"
              onClick={() => setTab(item.key)}
              className={cx(
                'flex items-center gap-1.5 border-b-2 px-2.5 py-2 text-[11.5px] transition-colors',
                active ? 'border-accent font-medium text-ink' : 'border-transparent text-muted hover:text-ink',
              )}
            >
              <Icon size={12} />
              {item.label}
            </button>
          );
        })}
      </div>

      <div className="flex-1 overflow-y-auto px-3.5 py-3">
        {tab === 'activity' ? <ActivityTab agentId={agentId} detail={detail} /> : null}
        {tab === 'context' ? <ContextTab detail={detail} /> : null}
        {tab === 'files' ? <DiffTab agentId={agentId} detail={detail} /> : null}
      </div>

      <div className="border-t border-line px-3.5 py-2">
        <div className="flex items-center justify-between text-[10px] text-muted">
          <span className="truncate" title={agent?.task_description}>
            {agent ? t('think.label', { level: thinkLabel(lang, agent.thinking_level) }) : ''}
          </span>
          <span>{agent ? t('insp.reasoningTokens', { count: formatTokens(agent.reasoning_tokens) }) : ''}</span>
        </div>
      </div>
    </aside>
  );
}
