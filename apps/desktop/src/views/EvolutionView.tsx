import { useEffect, useMemo, useState } from 'react';
import { BookOpen, Brain, Check, RotateCcw, Sparkles } from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import type { EvolutionRecord } from '../types';
import { Badge, Button, Card, EmptyState, SectionTitle, TextArea, cx, relativeTime } from '../components/ui';

function RecordRow({
  record,
  active,
  onSelect,
}: {
  record: EvolutionRecord;
  active: boolean;
  onSelect: () => void;
}) {
  const Icon = record.record_type === 'archetype_prompt' ? Brain : BookOpen;
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cx(
        'flex w-full items-start gap-2 rounded-lg px-2.5 py-2 text-left transition-colors focus-ring',
        active ? 'bg-raised' : 'hover:bg-raised/60',
      )}
    >
      <Icon size={13} className={cx('mt-0.5 shrink-0', active ? 'text-accent' : 'text-muted')} />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className={cx('truncate text-[12px]', active ? 'font-medium text-ink' : 'text-ink/90')}>
            {record.title}
          </span>
          <span className="shrink-0 font-mono text-[10px] text-muted">v{record.version}</span>
        </span>
        <span className="mt-0.5 block truncate text-[10.5px] text-muted">{record.key_name}</span>
      </span>
    </button>
  );
}

function Editor({ record }: { record: EvolutionRecord }) {
  const t = useT();
  const lang = useApp((state) => state.lang);
  const saveEvolution = useApp((state) => state.saveEvolution);
  const [draft, setDraft] = useState(record.content);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDraft(record.content);
  }, [record.id, record.content]);

  const dirty = draft !== record.content;

  async function save() {
    setSaving(true);
    try {
      await saveEvolution(record.id, draft);
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-3.5 py-2.5">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-[13px] font-semibold text-ink">{record.title}</h3>
          <p className="mt-0.5 flex items-center gap-1.5 text-[10.5px] text-muted">
            <code className="rounded bg-raised px-1 font-mono">{record.key_name}</code>
            <span>·</span>
            <span>v{record.version}</span>
            <span>·</span>
            <span>{relativeTime(record.updated_at, lang)}</span>
          </p>
        </div>
        <div className="flex items-center gap-1.5">
          {dirty ? <Badge tone="warn">{t('evo.unsaved')}</Badge> : null}
          <Button size="sm" variant="ghost" onClick={() => setDraft(record.content)} disabled={!dirty}>
            <RotateCcw size={12} />
            {t('evo.revert')}
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => void save()}
            disabled={!dirty || saving || !draft.trim()}
          >
            <Check size={12} />
            {saving ? t('common.saving') : t('evo.saveVersion')}
          </Button>
        </div>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-2 p-3.5">
        <p className="text-[11px] text-muted">
          {t('evo.lastChange', { note: record.changelog || t('evo.emptyNote') })}
        </p>
        <TextArea
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          spellCheck={false}
          className="min-h-[320px] flex-1 font-mono text-[11.5px] leading-relaxed"
        />
      </div>
    </Card>
  );
}

/** Skills & Evolution: versioned role prompts and distilled playbooks. */
export function EvolutionView() {
  const t = useT();
  const evolution = useApp((state) => state.evolution);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const prompts = useMemo(
    () => evolution.filter((record) => record.record_type === 'archetype_prompt'),
    [evolution],
  );
  const playbooks = useMemo(
    () => evolution.filter((record) => record.record_type === 'playbook_skill'),
    [evolution],
  );

  const selected = useMemo(
    () => evolution.find((record) => record.id === selectedId) ?? prompts[0] ?? playbooks[0] ?? null,
    [evolution, playbooks, prompts, selectedId],
  );

  return (
    <div className="flex h-full flex-col overflow-hidden">
      <div className="mx-auto flex w-full max-w-[1180px] flex-1 flex-col gap-4 overflow-hidden px-6 py-6">
        <header>
          <h1 className="text-[19px] font-semibold tracking-tight text-ink">{t('evo.title')}</h1>
          <p className="mt-1 max-w-[700px] text-[12.5px] leading-relaxed text-muted">{t('evo.subtitle')}</p>
        </header>

        {evolution.length === 0 ? (
          <EmptyState
            icon={<Sparkles size={20} />}
            title={t('evo.empty')}
            description={t('evo.emptyBody')}
          />
        ) : (
          <div className="grid min-h-0 flex-1 grid-cols-1 gap-4 lg:grid-cols-[300px_1fr]">
            <div className="flex min-h-0 flex-col gap-4 overflow-y-auto pr-1">
              <div>
                <SectionTitle title={t('evo.prompts')} action={<Badge>{prompts.length}</Badge>} />
                <div className="space-y-0.5">
                  {prompts.map((record) => (
                    <RecordRow
                      key={record.id}
                      record={record}
                      active={selected?.id === record.id}
                      onSelect={() => setSelectedId(record.id)}
                    />
                  ))}
                </div>
              </div>

              <div>
                <SectionTitle title={t('evo.playbooks')} action={<Badge>{playbooks.length}</Badge>} />
                {playbooks.length === 0 ? (
                  <p className="px-2.5 text-[11px] leading-relaxed text-muted">{t('evo.noPlaybooks')}</p>
                ) : (
                  <div className="space-y-0.5">
                    {playbooks.map((record) => (
                      <RecordRow
                        key={record.id}
                        record={record}
                        active={selected?.id === record.id}
                        onSelect={() => setSelectedId(record.id)}
                      />
                    ))}
                  </div>
                )}
              </div>
            </div>

            {selected ? <Editor record={selected} /> : null}
          </div>
        )}
      </div>
    </div>
  );
}
