import { useState } from 'react';
import type { Protocol, ProviderView } from '../types';
import { useApp } from '../store';
import { useT } from '../useT';
import { Button, Field, Select, TextInput, Toggle } from './ui';

const PROTOCOLS: Array<{ value: Protocol; labelKey: string; hintKey: string }> = [
  { value: 'openai_compat', labelKey: 'models.proto.openai', hintKey: 'models.hint.openai' },
  { value: 'anthropic', labelKey: 'models.proto.anthropic', hintKey: 'models.hint.anthropic' },
  { value: 'gemini', labelKey: 'models.proto.gemini', hintKey: 'models.hint.gemini' },
  { value: 'ollama', labelKey: 'models.proto.ollama', hintKey: 'models.hint.ollama' },
];

const DEFAULT_BASE: Record<Protocol, string> = {
  openai_compat: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com/v1',
  gemini: 'https://generativelanguage.googleapis.com/v1beta',
  ollama: 'http://127.0.0.1:11434',
};

/** Create or edit a provider. */
export function ProviderForm({ provider, onClose }: { provider?: ProviderView; onClose: () => void }) {
  const t = useT();
  const saveProvider = useApp((state) => state.saveProvider);

  const [name, setName] = useState(provider?.name ?? '');
  const [protocol, setProtocol] = useState<Protocol>(provider?.protocol ?? 'openai_compat');
  const [baseUrl, setBaseUrl] = useState(provider?.base_url ?? DEFAULT_BASE.openai_compat);
  const [apiKey, setApiKey] = useState('');
  const [enabled, setEnabled] = useState(provider?.is_enabled ?? true);
  const [saving, setSaving] = useState(false);

  const hint = PROTOCOLS.find((item) => item.value === protocol);

  async function submit() {
    setSaving(true);
    try {
      const ok = await saveProvider({
        id: provider?.id,
        name: name.trim(),
        protocol,
        base_url: baseUrl.trim() || DEFAULT_BASE[protocol],
        api_key: apiKey.trim() || undefined,
        is_enabled: enabled,
        search_engine_config: provider?.search_engine_config,
      });
      if (ok) onClose();
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="rounded-xl border border-accent/30 bg-surface p-3.5 shadow-card animate-fade-in">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-[13px] font-semibold text-ink">
          {provider ? t('models.editTitle', { name: provider.name }) : t('models.addTitle')}
        </h3>
        <Button size="sm" variant="ghost" onClick={onClose}>
          {t('common.cancel')}
        </Button>
      </div>

      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
        <Field label={t('models.displayName')}>
          <TextInput value={name} onChange={(event) => setName(event.target.value)} placeholder="OpenAI" />
        </Field>

        <Field label={t('models.protocol')} hint={hint ? t(hint.hintKey) : undefined}>
          <Select
            value={protocol}
            onChange={(event) => {
              const next = event.target.value as Protocol;
              setProtocol(next);
              if (!provider) setBaseUrl(DEFAULT_BASE[next]);
            }}
            className="h-9 text-[13px]"
          >
            {PROTOCOLS.map((item) => (
              <option key={item.value} value={item.value}>
                {t(item.labelKey)}
              </option>
            ))}
          </Select>
        </Field>

        <Field label={t('models.baseUrl')}>
          <TextInput
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
            placeholder={DEFAULT_BASE[protocol]}
          />
        </Field>

        <Field
          label={t('models.apiKey')}
          hint={
            provider?.has_api_key
              ? t('models.keyKept', { masked: provider.api_key_masked })
              : t('models.keyOptional')
          }
        >
          <TextInput
            type="password"
            value={apiKey}
            onChange={(event) => setApiKey(event.target.value)}
            placeholder={provider?.has_api_key ? '••••••••' : 'sk-…'}
            autoComplete="off"
          />
        </Field>
      </div>

      <div className="mt-2.5">
        <Toggle
          checked={enabled}
          onChange={setEnabled}
          label={t('models.enabled')}
          description={t('models.enabledHint')}
        />
      </div>

      <div className="mt-3 flex justify-end gap-2">
        <Button size="sm" variant="secondary" onClick={onClose}>
          {t('common.discard')}
        </Button>
        <Button size="sm" variant="primary" onClick={() => void submit()} disabled={saving || !name.trim()}>
          {saving ? t('common.saving') : t('models.saveProvider')}
        </Button>
      </div>
    </div>
  );
}
