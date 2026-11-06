import { useState } from 'react';
import { Check, Eye, Loader2, RefreshCw, Settings2, Trash2, Wrench } from 'lucide-react';
import type { ProviderView } from '../types';
import { useApp } from '../store';
import { useT } from '../useT';
import { Badge, Button, Card, Field, SectionTitle, Select, TextInput } from './ui';

function ProtocolBadge({ protocol }: { protocol: string }) {
  const t = useT();
  const key =
    protocol === 'openai_compat'
      ? 'models.proto.openai'
      : protocol === 'anthropic'
        ? 'models.proto.anthropic'
        : protocol === 'gemini'
          ? 'models.proto.gemini'
          : 'models.proto.ollama';
  return <Badge tone="neutral">{t(key)}</Badge>;
}

function ProviderCard({
  provider,
  onEdit,
}: {
  provider: ProviderView;
  onEdit: (provider: ProviderView) => void;
}) {
  const t = useT();
  const pullModels = useApp((state) => state.pullModels);
  const profileModels = useApp((state) => state.profileModels);
  const removeProvider = useApp((state) => state.removeProvider);
  const busy = useApp((state) => state.busy[provider.id]);

  const hasSearchApi =
    provider.search_engine_config.tavily_api_key ||
    provider.search_engine_config.brave_api_key ||
    provider.search_engine_config.serper_api_key ||
    provider.search_engine_config.searxng_endpoint;

  return (
    <Card className="p-3.5">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <h3 className="truncate text-[13px] font-semibold text-ink">{provider.name}</h3>
            <ProtocolBadge protocol={provider.protocol} />
            {!provider.is_enabled ? <Badge tone="warn">{t('models.disabled')}</Badge> : null}
          </div>
          <p className="mt-0.5 truncate text-[11px] text-muted" title={provider.base_url}>
            {provider.base_url}
          </p>
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
            <Badge tone={provider.model_count > 0 ? 'accent' : 'neutral'}>
              {t('models.modelCount', { count: provider.model_count })}
            </Badge>
            <Badge tone={provider.has_api_key ? 'ok' : 'warn'}>
              {provider.has_api_key ? t('models.keySet') : t('models.noKey')}
            </Badge>
            <Badge tone={hasSearchApi ? 'accent' : 'neutral'}>
              {hasSearchApi ? t('models.searchApi') : t('models.scraper')}
            </Badge>
          </div>
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5">
        <Button size="sm" variant="secondary" onClick={() => void pullModels(provider.id)} disabled={busy}>
          {busy ? <Loader2 size={12} className="animate-spin" /> : <RefreshCw size={12} />}
          {t('models.pullList')}
        </Button>
        <Button size="sm" variant="secondary" onClick={() => void profileModels(provider.id)} disabled={busy}>
          <Wrench size={12} />
          {t('models.runProfiler')}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onEdit(provider)}>
          <Settings2 size={12} />
          {t('models.edit')}
        </Button>
        <Button size="sm" variant="ghost" className="ml-auto" onClick={() => void removeProvider(provider.id)}>
          <Trash2 size={12} />
        </Button>
      </div>
    </Card>
  );
}

/** Web-search credentials, stored per provider. */
export function SearchPreferences({ providers }: { providers: ProviderView[] }) {
  const t = useT();
  const saveSearchConfig = useApp((state) => state.saveSearchConfig);
  const [providerId, setProviderId] = useState(providers[0]?.id ?? '');
  const selected = providers.find((item) => item.id === providerId) ?? providers[0];

  const [tavily, setTavily] = useState(selected?.search_engine_config.tavily_api_key ?? '');
  const [brave, setBrave] = useState(selected?.search_engine_config.brave_api_key ?? '');
  const [serper, setSerper] = useState(selected?.search_engine_config.serper_api_key ?? '');
  const [searxng, setSearxng] = useState(selected?.search_engine_config.searxng_endpoint ?? '');
  const [preferred, setPreferred] = useState(selected?.search_engine_config.preferred_engine ?? 'auto');

  if (!providers.length) {
    return (
      <Card className="p-3.5">
        <p className="text-[12px] text-muted">{t('toast.pullFirst')}</p>
      </Card>
    );
  }

  function loadFor(id: string) {
    const next = providers.find((item) => item.id === id);
    setProviderId(id);
    setTavily(next?.search_engine_config.tavily_api_key ?? '');
    setBrave(next?.search_engine_config.brave_api_key ?? '');
    setSerper(next?.search_engine_config.serper_api_key ?? '');
    setSearxng(next?.search_engine_config.searxng_endpoint ?? '');
    setPreferred(next?.search_engine_config.preferred_engine ?? 'auto');
  }

  return (
    <Card className="p-3.5">
      <p className="mb-2.5 text-[11.5px] leading-relaxed text-muted">{t('models.searchBody')}</p>

      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
        <Field label={t('models.appliesTo')}>
          <Select value={providerId} onChange={(event) => loadFor(event.target.value)} className="h-9 text-[13px]">
            {providers.map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.name}
              </option>
            ))}
          </Select>
        </Field>

        <Field label={t('models.preferred')} hint={t('models.preferredHint')}>
          <Select
            value={preferred}
            onChange={(event) => setPreferred(event.target.value)}
            className="h-9 text-[13px]"
          >
            {['auto', 'tavily', 'brave', 'serper', 'searxng', 'duckduckgo', 'bing'].map((engine) => (
              <option key={engine} value={engine}>
                {engine}
              </option>
            ))}
          </Select>
        </Field>

        <Field label={t('models.tavily')}>
          <TextInput value={tavily} onChange={(event) => setTavily(event.target.value)} placeholder="tvly-…" />
        </Field>

        <Field label={t('models.brave')}>
          <TextInput value={brave} onChange={(event) => setBrave(event.target.value)} placeholder="BSA…" />
        </Field>

        <Field label={t('models.serper')}>
          <TextInput value={serper} onChange={(event) => setSerper(event.target.value)} placeholder="…" />
        </Field>

        <Field label={t('models.searx')}>
          <TextInput
            value={searxng}
            onChange={(event) => setSearxng(event.target.value)}
            placeholder="https://searx.example.com"
          />
        </Field>
      </div>

      <div className="mt-3 flex justify-end">
        <Button
          size="sm"
          variant="primary"
          onClick={() =>
            void saveSearchConfig(selected.id, {
              tavily_api_key: tavily.trim() || undefined,
              brave_api_key: brave.trim() || undefined,
              serper_api_key: serper.trim() || undefined,
              searxng_endpoint: searxng.trim() || undefined,
              preferred_engine: preferred === 'auto' ? undefined : preferred,
            })
          }
        >
          <Check size={12} />
          {t('models.saveSearch')}
        </Button>
      </div>
    </Card>
  );
}

export function ProviderGrid({
  providers,
  onEdit,
}: {
  providers: ProviderView[];
  onEdit: (provider: ProviderView | null) => void;
}) {
  const t = useT();
  const profileModels = useApp((state) => state.profileModels);
  const profiling = useApp((state) => state.profiling);

  return (
    <section>
      <SectionTitle
        title={t('models.providers')}
        hint={t('models.providersHint')}
        action={
          <div className="flex items-center gap-2">
            {profiling.active ? (
              <span className="flex items-center gap-1.5 text-[11px] text-accent">
                <Loader2 size={11} className="animate-spin" />
                {profiling.message}
              </span>
            ) : null}
            <Button size="sm" variant="secondary" onClick={() => void profileModels()} disabled={!providers.length}>
              <Eye size={12} />
              {t('models.profileAll')}
            </Button>
            <Button size="sm" variant="primary" onClick={() => onEdit(null)}>
              {t('models.addProvider')}
            </Button>
          </div>
        }
      />

      {providers.length === 0 ? (
        <Card className="p-5 text-center">
          <p className="text-[13px] font-medium text-ink">{t('models.noProviders')}</p>
          <p className="mx-auto mt-1 max-w-md text-[11.5px] leading-relaxed text-muted">
            {t('models.noProvidersBody')}
          </p>
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-2.5 lg:grid-cols-2">
          {providers.map((provider) => (
            <ProviderCard key={provider.id} provider={provider} onEdit={onEdit} />
          ))}
        </div>
      )}
    </section>
  );
}
