import { useEffect, useMemo, useState } from 'react';
import { Search, Users } from 'lucide-react';
import { archetypeLabel, thinkLabel } from '../i18n';
import { useApp } from '../store';
import { useT } from '../useT';
import type { AgentStatus, AgentSummary } from '../types';
import { Badge, EmptyState, StatusDot, cx, formatDuration, formatTokens } from '../components/ui';

type Filter = 'all' | 'running' | 'waiting_dependency' | 'completed' | 'failed';

const FILTERS: Array<{ key: Filter; labelKey: string }> = [
  { key: 'all', labelKey: 'swarm.filter.all' },
  { key: 'running', labelKey: 'swarm.filter.running' },
  { key: 'waiting_dependency', labelKey: 'swarm.filter.waiting' },
  { key: 'completed', labelKey: 'swarm.filter.completed' },
  { key: 'failed', labelKey: 'swarm.filter.failed' },
];

function matchesFilter(agent: AgentSummary, filter: Filter): boolean {
  if (filter === 'all') return true;
  if (filter === 'running') return agent.status === 'running' || agent.status === 'queued';
  return agent.status === (filter as AgentStatus);
}

function AgentCard({ agent, tick, snapshotAt }: { agent: AgentSummary; tick: number; snapshotAt: number }) {
  const t = useT();
  const lang = useApp((state) => state.lang);
  const live = useApp((state) => state.live[agent.id]);
  const openInspector = useApp((state) => state.openInspector);
  const inspectorAgentId = useApp((state) => state.inspectorAgentId);

  const running = agent.status === 'running' || agent.status === 'queued';
  const activity =
    live?.activity ||
    agent.activity ||
    (agent.status === 'waiting_dependency' ? t('swarm.waitingUpstream') : '') ||
    (agent.status === 'queued' ? t('swarm.queued') : '') ||
    agent.output_preview?.split('\n')[0] ||
    '—';

  const tokens = live
    ? live.promptTokens + live.completionTokens
    : agent.prompt_tokens + agent.completion_tokens;

  // Elapsed is measured at snapshot time; add the time since to keep it live.
  const elapsed = running ? agent.elapsed_ms + Math.max(0, tick - snapshotAt) : agent.elapsed_ms;

  return (
    <button
      type="button"
      onClick={() => void openInspector(agent.id)}
      className={cx(
        'group flex flex-col gap-1.5 rounded-lg border bg-surface p-2.5 text-left transition-all hover:border-accent/40 hover:shadow-card focus-ring',
        inspectorAgentId === agent.id ? 'border-accent/60 ring-1 ring-accent/20' : 'border-line',
      )}
    >
      <div className="flex items-center gap-1.5">
        <StatusDot status={agent.status} />
        <span className="min-w-0 flex-1 truncate text-[12.5px] font-medium text-ink" title={agent.role_name}>
          {agent.role_name}
        </span>
        {agent.status === 'failed' ? <Badge tone="danger">{t('swarm.filter.failed')}</Badge> : null}
      </div>

      <div className="flex flex-wrap items-center gap-1">
        <Badge title={agent.model_id || 'auto'}>
          <span className="max-w-[132px] truncate">{agent.model_name || 'auto'}</span>
        </Badge>
        <Badge tone={agent.thinking_level === 'off' ? 'neutral' : 'accent'}>
          {t('think.label', { level: thinkLabel(lang, agent.thinking_level) })}
        </Badge>
        <Badge tone="neutral">{archetypeLabel(lang, agent.archetype)}</Badge>
      </div>

      <p
        className={cx(
          'line-clamp-1 text-[11px] leading-relaxed',
          agent.status === 'failed' ? 'text-danger' : 'text-muted',
        )}
        title={agent.error_message ?? activity}
      >
        {agent.status === 'failed' ? agent.error_message ?? activity : activity}
      </p>

      <div className="flex items-center justify-between text-[10px] text-muted">
        <span>{formatDuration(elapsed)}</span>
        <span>{formatTokens(tokens)} tok</span>
      </div>
    </button>
  );
}

/** Responsive grid of compact agent cards with filters and search. */
export function SwarmGrid() {
  const t = useT();
  const snapshot = useApp((state) => state.snapshot);
  const [filter, setFilter] = useState<Filter>('all');
  const [query, setQuery] = useState('');
  const [tick, setTick] = useState(() => Date.now());
  const [snapshotAt, setSnapshotAt] = useState(() => Date.now());

  useEffect(() => {
    const timer = setInterval(() => setTick(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (snapshot) setSnapshotAt(Date.now());
  }, [snapshot]);

  const agents = snapshot?.agents ?? [];

  const counts = useMemo(() => {
    const map: Record<Filter, number> = {
      all: agents.length,
      running: 0,
      waiting_dependency: 0,
      completed: 0,
      failed: 0,
    };
    for (const agent of agents) {
      if (agent.status === 'running' || agent.status === 'queued') map.running += 1;
      else if (agent.status === 'waiting_dependency') map.waiting_dependency += 1;
      else if (agent.status === 'completed') map.completed += 1;
      else if (agent.status === 'failed') map.failed += 1;
    }
    return map;
  }, [agents]);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return agents.filter((agent) => {
      if (!matchesFilter(agent, filter)) return false;
      if (!needle) return true;
      return (
        agent.role_name.toLowerCase().includes(needle) ||
        agent.archetype.toLowerCase().includes(needle) ||
        agent.model_name.toLowerCase().includes(needle) ||
        agent.task_description.toLowerCase().includes(needle)
      );
    });
  }, [agents, filter, query]);

  return (
    <section>
      <div className="mb-2.5 flex flex-wrap items-center gap-2">
        <h2 className="mr-1 flex items-center gap-1.5 text-[12px] font-semibold uppercase tracking-wide text-muted">
          <Users size={13} />
          {t('swarm.title')}
          <Badge>{agents.length}</Badge>
        </h2>

        <div className="flex flex-wrap items-center gap-1">
          {FILTERS.map((item) => (
            <button
              key={item.key}
              type="button"
              onClick={() => setFilter(item.key)}
              className={cx(
                'rounded-md px-2 py-[3px] text-[11.5px] transition-colors',
                filter === item.key ? 'bg-raised font-medium text-ink' : 'text-muted hover:text-ink',
              )}
            >
              {t(item.labelKey)}
              {counts[item.key] > 0 ? <span className="ml-1 text-muted/70">{counts[item.key]}</span> : null}
            </button>
          ))}
        </div>

        <div className="relative ml-auto">
          <Search size={12} className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted" />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('swarm.search')}
            className="h-7 w-[180px] rounded-md border border-line bg-surface pl-[1.65rem] pr-2 text-[11.5px] outline-none placeholder:text-muted/70 focus:border-accent/50"
          />
        </div>
      </div>

      {agents.length === 0 ? (
        <EmptyState icon={<Users size={20} />} title={t('swarm.empty')} description={t('swarm.emptyBody')} />
      ) : visible.length === 0 ? (
        <p className="rounded-lg border border-dashed border-line px-3 py-6 text-center text-[12px] text-muted">
          {t('swarm.noMatch')}
        </p>
      ) : (
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
          {visible.map((agent) => (
            <AgentCard key={agent.id} agent={agent} tick={tick} snapshotAt={snapshotAt} />
          ))}
        </div>
      )}
    </section>
  );
}
