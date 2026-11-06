import { useEffect, useMemo, useState } from 'react';
import {
  Brain,
  ChevronDown,
  ChevronRight,
  CircleStop,
  Copy,
  Download,
  FolderTree,
  Layers,
  Network,
  Pause,
  Play,
  ShieldCheck,
  Sparkles,
} from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import { Markdown } from '../components/Markdown';
import { Composer } from '../components/Composer';
import { SwarmGrid } from './SwarmGrid';
import {
  Badge,
  Button,
  Card,
  IconButton,
  StatusBadge,
  formatBytes,
  formatDuration,
  formatTokens,
  relativeTime,
} from '../components/ui';

function TopBar() {
  const t = useT();
  const lang = useApp((state) => state.lang);
  const snapshot = useApp((state) => state.snapshot);
  const controlSession = useApp((state) => state.controlSession);
  const externalChanges = useApp((state) => state.externalChanges);
  const clearExternalChanges = useApp((state) => state.clearExternalChanges);
  if (!snapshot) return null;

  const { session, agents, totals } = snapshot;
  const active = session.status === 'running' || session.status === 'planning';
  const paused = session.status === 'paused';

  return (
    <div className="flex flex-wrap items-center gap-2 border-b border-line bg-surface/80 px-4 py-2.5 backdrop-blur">
      <div className="flex min-w-0 items-center gap-1.5" title={session.workspace_path}>
        <FolderTree size={13} className="shrink-0 text-muted" />
        <span className="max-w-[280px] truncate text-[11.5px] text-muted">{session.workspace_path}</span>
      </div>

      <div className="flex flex-wrap items-center gap-1">
        <Badge tone="neutral">
          <Network size={10} />
          {t(`topo.${session.topology_mode}`)}
        </Badge>
        <Badge tone="neutral">
          <Layers size={10} />
          {t(`ctx.${session.context_strategy}`)}
        </Badge>
        <Badge tone="neutral">
          <ShieldCheck size={10} />
          {t(`iso.${session.isolation_strategy}`)}
        </Badge>
      </div>

      <div className="ml-auto flex items-center gap-2">
        {externalChanges.count > 0 ? (
          <button
            type="button"
            onClick={clearExternalChanges}
            title={`${t('stage.diskHint')}\n${externalChanges.paths.slice(-8).join('\n')}`}
          >
            <Badge tone="warn">{t('stage.diskChanges', { count: externalChanges.count })}</Badge>
          </button>
        ) : null}
        <span className="text-[11px] text-muted">
          {t('stage.agentsTok', {
            agents: agents.length,
            tokens: formatTokens(totals.prompt_tokens + totals.completion_tokens),
          })}
        </span>
        <StatusBadge status={session.status} />
        {active ? (
          <Button size="sm" variant="secondary" onClick={() => void controlSession('pause')}>
            <Pause size={12} />
            {t('status.paused')}
          </Button>
        ) : null}
        {paused ? (
          <Button size="sm" variant="secondary" onClick={() => void controlSession('resume')}>
            <Play size={12} />
            {t('status.running')}
          </Button>
        ) : null}
        {!['completed', 'failed'].includes(session.status) ? (
          <Button size="sm" variant="secondary" onClick={() => void controlSession('cancel')}>
            <CircleStop size={12} />
            {t('status.cancelled')}
          </Button>
        ) : null}
        <span className="sr-only">{lang}</span>
      </div>
    </div>
  );
}

function ObjectiveCard() {
  const lang = useApp((state) => state.lang);
  const snapshot = useApp((state) => state.snapshot);
  if (!snapshot) return null;
  const { session, attachments } = snapshot;

  return (
    <Card className="p-3.5">
      <div className="mb-1.5 flex items-start justify-between gap-3">
        <h1 className="text-[15px] font-semibold leading-snug tracking-tight text-ink">{session.title}</h1>
        <span className="shrink-0 text-[10.5px] text-muted">{relativeTime(session.created_at, lang)}</span>
      </div>
      <p className="whitespace-pre-wrap text-[12.5px] leading-relaxed text-muted">{session.user_objective}</p>

      {attachments.length ? (
        <div className="mt-2.5 flex flex-wrap gap-1.5">
          {attachments.map((attachment) => (
            <div
              key={attachment.id}
              className="flex items-center gap-2 rounded-lg border border-line bg-canvas px-2 py-1.5"
              title={attachment.file_path}
            >
              {attachment.base64_thumbnail && attachment.mime_type.startsWith('image/') ? (
                <img
                  src={`data:${attachment.mime_type};base64,${attachment.base64_thumbnail}`}
                  alt={attachment.file_name}
                  className="h-8 w-8 rounded object-cover"
                />
              ) : (
                <span className="flex h-8 w-8 items-center justify-center rounded bg-raised text-[9px] font-semibold uppercase text-muted">
                  {(attachment.file_name.split('.').pop() ?? 'file').slice(0, 4)}
                </span>
              )}
              <span>
                <span className="block max-w-[160px] truncate text-[11.5px] font-medium text-ink">
                  {attachment.file_name}
                </span>
                <span className="block text-[10px] text-muted">{formatBytes(attachment.file_size_bytes)}</span>
              </span>
            </div>
          ))}
        </div>
      ) : null}
    </Card>
  );
}

function OrchestratorTrace() {
  const t = useT();
  const snapshot = useApp((state) => state.snapshot);
  const orchestrator = useMemo(
    () => snapshot?.agents.find((agent) => agent.archetype === 'meta_orchestrator'),
    [snapshot],
  );
  const trace = useApp((state) => (orchestrator ? state.live[orchestrator.id]?.thinking ?? '' : ''));
  const liveActivity = useApp((state) => (orchestrator ? state.live[orchestrator.id]?.activity ?? '' : ''));
  const [open, setOpen] = useState(false);

  if (!orchestrator) return null;
  const activity = liveActivity || orchestrator.activity;

  return (
    <Card className="overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-center gap-2 px-3.5 py-2.5 text-left transition-colors hover:bg-raised/50"
      >
        <Brain size={13} className="shrink-0 text-accent" />
        <span className="text-[12.5px] font-medium text-ink">{t('stage.orchestrator')}</span>
        <span className="min-w-0 flex-1 truncate text-[11.5px] text-muted">
          {activity || t('stage.planning')}
        </span>
        <span className="shrink-0 text-[10.5px] text-muted">{formatDuration(orchestrator.elapsed_ms)}</span>
        {open ? (
          <ChevronDown size={13} className="text-muted" />
        ) : (
          <ChevronRight size={13} className="text-muted" />
        )}
      </button>

      {open ? (
        <div className="border-t border-line px-3.5 py-3">
          <p className="mb-1 text-[10.5px] font-semibold uppercase tracking-wider text-muted">
            {t('stage.thinkingTitle')}
          </p>
          {trace ? (
            <pre className="max-h-64 overflow-y-auto whitespace-pre-wrap rounded-lg bg-canvas p-2.5 font-mono text-[11.5px] leading-relaxed text-muted">
              {trace}
            </pre>
          ) : (
            <p className="text-[11.5px] text-muted">{t('stage.noReasoning')}</p>
          )}
          <p className="mt-2 text-[11px] text-muted">{orchestrator.task_description}</p>
        </div>
      ) : null}
    </Card>
  );
}

function SharedFindings() {
  const t = useT();
  const snapshot = useApp((state) => state.snapshot);
  const entries = snapshot?.blackboard ?? [];
  if (!entries.length) return null;

  return (
    <Card className="p-3.5">
      <div className="mb-2 flex items-center gap-1.5">
        <Sparkles size={13} className="text-accent" />
        <h2 className="text-[12px] font-semibold uppercase tracking-wide text-muted">
          {t('stage.sharedFindings')}
        </h2>
        <Badge>{entries.length}</Badge>
      </div>
      <ul className="space-y-1.5">
        {entries.slice(0, 6).map((entry) => (
          <li key={entry.id} className="flex items-start gap-2 text-[11.5px] leading-relaxed">
            <Badge tone="accent">{entry.topic_tag}</Badge>
            <span className="min-w-0 flex-1 text-muted">{entry.summary}</span>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function DeliverableCard() {
  const t = useT();
  const snapshot = useApp((state) => state.snapshot);
  const pushToast = useApp((state) => state.pushToast);
  const [copied, setCopied] = useState(false);

  const summary = snapshot?.final_summary ?? null;
  if (!snapshot || !summary) return null;

  async function copy() {
    try {
      await navigator.clipboard.writeText(summary ?? '');
      setCopied(true);
      setTimeout(() => setCopied(false), 1600);
    } catch {
      pushToast('error', t('toast.clipboardDenied'));
    }
  }

  function download() {
    const blob = new Blob([summary ?? ''], { type: 'text/markdown;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `${(snapshot?.session.title ?? 'reverie-report').replace(/[^\w.-]+/g, '-')}.md`;
    document.body.appendChild(anchor);
    anchor.click();
    anchor.remove();
    URL.revokeObjectURL(url);
  }

  return (
    <Card className="overflow-hidden">
      <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5">
        <Sparkles size={13} className="text-ok" />
        <h2 className="text-[12.5px] font-semibold text-ink">{t('stage.finalDeliverable')}</h2>
        <div className="ml-auto flex items-center gap-1">
          <IconButton label={t('stage.copyReport')} onClick={() => void copy()}>
            <Copy size={13} className={copied ? 'text-ok' : undefined} />
          </IconButton>
          <IconButton label={t('stage.downloadMd')} onClick={download}>
            <Download size={13} />
          </IconButton>
        </div>
      </div>
      <div className="max-h-[520px] overflow-y-auto px-4 py-3.5">
        <Markdown>{summary}</Markdown>
      </div>
    </Card>
  );
}

/** The task execution workspace. */
export function TaskStage() {
  const t = useT();
  const snapshot = useApp((state) => state.snapshot);
  const reflectSession = useApp((state) => state.reflectSession);

  useEffect(() => {
    // Keep the stage fresh while a run is in flight even if no event arrives.
    const timer = setInterval(() => {
      const state = useApp.getState();
      if (state.snapshot && (state.snapshot.session.status === 'running' || state.snapshot.active)) {
        void state.refreshSnapshot();
      }
    }, 6000);
    return () => clearInterval(timer);
  }, []);

  if (!snapshot) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-[12.5px] text-muted">{t('stage.loading')}</p>
      </div>
    );
  }

  const active = snapshot.session.status === 'running' || snapshot.session.status === 'planning';
  const finished = ['completed', 'failed'].includes(snapshot.session.status);

  return (
    <div className="flex h-full min-w-0 flex-col">
      <TopBar />

      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-[980px] flex-col gap-3 px-4 py-4">
          <ObjectiveCard />
          <OrchestratorTrace />
          <SwarmGrid />
          <SharedFindings />
          <DeliverableCard />

          {finished ? (
            <div className="flex items-center justify-between rounded-lg border border-line bg-surface px-3.5 py-2.5">
              <p className="text-[11.5px] text-muted">{t('stage.finished')}</p>
              <Button size="sm" variant="secondary" onClick={() => void reflectSession(snapshot.session.id)}>
                <Sparkles size={12} />
                {t('stage.runReflection')}
              </Button>
            </div>
          ) : null}
        </div>
      </div>

      <div className="border-t border-line bg-canvas/80 px-4 py-3 backdrop-blur">
        <div className="mx-auto max-w-[980px]">
          {active ? (
            <div className="flex items-center gap-2 rounded-lg border border-line bg-surface px-3 py-2">
              <span className="h-1.5 w-1.5 animate-pulseDot rounded-full bg-accent" />
              <p className="text-[11.5px] text-muted">{t('stage.working')}</p>
            </div>
          ) : (
            <Composer />
          )}
        </div>
      </div>
    </div>
  );
}
