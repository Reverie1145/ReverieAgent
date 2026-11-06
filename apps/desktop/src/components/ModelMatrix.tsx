import { useMemo, useState } from 'react';
import { Eye, EyeOff, Search } from 'lucide-react';
import type { ModelProfile, ProviderView } from '../types';
import { useApp } from '../store';
import { useT } from '../useT';
import { Badge, EmptyState, SectionTitle, Select, cx } from './ui';

function ScoreBar({ value }: { value: number }) {
  const pct = Math.max(0, Math.min(100, value * 10));
  return (
    <div className="flex items-center gap-1.5">
      <div className="h-1 w-10 overflow-hidden rounded-full bg-line">
        <div className="h-full rounded-full bg-accent" style={{ width: `${pct}%` }} />
      </div>
      <span className="w-7 text-right font-mono text-[10.5px] text-muted">{value.toFixed(1)}</span>
    </div>
  );
}

function Row({ model, providerName }: { model: ModelProfile; providerName: string }) {
  const t = useT();
  const updateModelSettings = useApp((state) => state.updateModelSettings);
  const [notesOpen, setNotesOpen] = useState(false);

  return (
    <tr className="border-t border-line align-top transition-colors hover:bg-raised/40">
      <td className="px-2.5 py-2">
        <p className="max-w-[220px] truncate text-[12px] font-medium text-ink" title={model.model_name}>
          {model.model_name}
        </p>
        <p className="text-[10.5px] text-muted">
          {providerName} · {(model.context_window / 1000).toFixed(0)}k ctx
        </p>
      </td>

      <td className="px-2.5 py-2">
        {model.supports_vision ? (
          <span className="inline-flex items-center gap-1 text-[11px] text-ok">
            <Eye size={11} /> {t('common.yes')}
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 text-[11px] text-muted">
            <EyeOff size={11} /> {t('common.no')}
          </span>
        )}
      </td>

      <td className="px-2.5 py-2">
        {model.supports_thinking ? (
          <Select
            value={model.default_thinking_level}
            onChange={(event) => void updateModelSettings(model.id, event.target.value, model.cost_tier)}
            className="h-6 w-[92px] text-[11px]"
          >
            {model.supported_thinking_levels.map((level) => (
              <option key={level} value={level}>
                {t(`think.${level}`)}
              </option>
            ))}
          </Select>
        ) : (
          <Badge tone="neutral">{t('common.none')}</Badge>
        )}
      </td>

      <td className="px-2.5 py-2 font-mono text-[11px] text-muted">{model.latency_ms} ms</td>

      <td className="px-2.5 py-2">
        <ScoreBar value={model.reasoning_score} />
      </td>
      <td className="px-2.5 py-2">
        <ScoreBar value={model.coding_score} />
      </td>
      <td className="px-2.5 py-2">
        <ScoreBar value={model.extraction_score} />
      </td>

      <td className="px-2.5 py-2">
        <Select
          value={String(model.cost_tier)}
          onChange={(event) =>
            void updateModelSettings(model.id, model.default_thinking_level, Number(event.target.value))
          }
          className="h-6 w-[104px] text-[11px]"
        >
          <option value="1">{t('models.cost1')}</option>
          <option value="2">{t('models.cost2')}</option>
          <option value="3">{t('models.cost3')}</option>
        </Select>
      </td>

      <td className="max-w-[240px] px-2.5 py-2">
        <button
          type="button"
          onClick={() => setNotesOpen((value) => !value)}
          className={cx('text-left text-[11px] leading-relaxed text-muted', notesOpen ? '' : 'line-clamp-2')}
          title={t('insp.expand')}
        >
          {model.task_fit_notes || '—'}
        </button>
      </td>
    </tr>
  );
}

/** The model capability matrix. */
export function ModelMatrix({ models, providers }: { models: ModelProfile[]; providers: ProviderView[] }) {
  const t = useT();
  const [query, setQuery] = useState('');
  const [onlyVision, setOnlyVision] = useState(false);

  const providerName = useMemo(() => {
    const map = new Map(providers.map((provider) => [provider.id, provider.name]));
    return (id: string) => map.get(id) ?? 'unknown';
  }, [providers]);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return models
      .filter((model) => (onlyVision ? model.supports_vision : true))
      .filter((model) =>
        needle
          ? model.model_name.toLowerCase().includes(needle) ||
            providerName(model.provider_id).toLowerCase().includes(needle)
          : true,
      );
  }, [models, onlyVision, providerName, query]);

  return (
    <section>
      <SectionTitle
        title={t('models.matrix')}
        hint={t('models.matrixHint')}
        action={
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => setOnlyVision((value) => !value)}
              className={cx(
                'inline-flex h-7 items-center gap-1 rounded-md border px-2 text-[11.5px] transition-colors',
                onlyVision
                  ? 'border-transparent bg-accentSoft text-accent'
                  : 'border-line text-muted hover:text-ink',
              )}
            >
              <Eye size={11} />
              {t('models.visionOnly')}
            </button>
            <div className="relative">
              <Search
                size={12}
                className="pointer-events-none absolute left-2 top-1/2 -translate-y-1/2 text-muted"
              />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t('models.filter')}
                className="h-7 w-[170px] rounded-md border border-line bg-surface pl-[1.6rem] pr-2 text-[11.5px] outline-none placeholder:text-muted/70 focus:border-accent/50"
              />
            </div>
          </div>
        }
      />

      {models.length === 0 ? (
        <EmptyState title={t('models.noModels')} description={t('models.noModelsBody')} />
      ) : (
        <div className="overflow-x-auto rounded-xl border border-line bg-surface">
          <table className="w-full min-w-[980px] border-collapse text-left">
            <thead>
              <tr className="bg-raised/60 text-[10.5px] uppercase tracking-wider text-muted">
                <th className="px-2.5 py-2 font-semibold">{t('models.col.model')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.vision')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.thinking')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.latency')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.reasoning')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.coding')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.extraction')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.cost')}</th>
                <th className="px-2.5 py-2 font-semibold">{t('models.col.notes')}</th>
              </tr>
            </thead>
            <tbody>
              {visible.map((model) => (
                <Row key={model.id} model={model} providerName={providerName(model.provider_id)} />
              ))}
            </tbody>
          </table>
          {visible.length === 0 ? (
            <p className="px-3 py-6 text-center text-[11.5px] text-muted">{t('models.noMatch')}</p>
          ) : null}
        </div>
      )}
    </section>
  );
}
