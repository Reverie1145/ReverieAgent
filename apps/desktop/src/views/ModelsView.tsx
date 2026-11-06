import { useState } from 'react';
import { useApp } from '../store';
import { useT } from '../useT';
import type { ProviderView } from '../types';
import { ProviderForm } from '../components/ProviderForm';
import { ProviderGrid, SearchPreferences } from '../components/ProviderGrid';
import { ModelMatrix } from '../components/ModelMatrix';
import { SectionTitle } from '../components/ui';

/** Models & Search: providers, profiling and the capability matrix. */
export function ModelsView() {
  const t = useT();
  const providers = useApp((state) => state.providers);
  const models = useApp((state) => state.models);
  const [editing, setEditing] = useState<ProviderView | null | undefined>(undefined);

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-[1180px] flex-col gap-6 px-6 py-6">
        <header>
          <h1 className="text-[19px] font-semibold tracking-tight text-ink">{t('models.title')}</h1>
          <p className="mt-1 max-w-[680px] text-[12.5px] leading-relaxed text-muted">{t('models.subtitle')}</p>
        </header>

        <ProviderGrid providers={providers} onEdit={(provider) => setEditing(provider)} />

        {editing !== undefined ? (
          <ProviderForm provider={editing ?? undefined} onClose={() => setEditing(undefined)} />
        ) : null}

        <section>
          <SectionTitle title={t('models.search')} hint={t('models.searchHint')} />
          <SearchPreferences providers={providers} />
        </section>

        <ModelMatrix models={models} providers={providers} />
      </div>
    </div>
  );
}
