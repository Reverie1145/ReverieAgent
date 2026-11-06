import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, Code2, Hammer, Terminal } from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import type { ToolDescriptor } from '../types';
import { Badge, Card, EmptyState, SectionTitle } from '../components/ui';

function ToolCard({ tool }: { tool: ToolDescriptor }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const parameters = tool.parameters as { properties?: Record<string, unknown>; required?: string[] };
  const fields = Object.keys(parameters?.properties ?? {});
  const required = parameters?.required ?? [];

  return (
    <Card className="p-3">
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <code className="rounded bg-raised px-1.5 py-0.5 font-mono text-[11.5px] font-medium text-ink">
              {tool.name}
            </code>
            {tool.is_dynamic ? (
              <Badge tone="accent">{t('tools.forgedBadge')}</Badge>
            ) : (
              <Badge tone="neutral">{t('tools.nativeBadge')}</Badge>
            )}
            {tool.interpreter ? <Badge>{tool.interpreter}</Badge> : null}
            {tool.usage_count > 0 ? <Badge>{t('tools.calls', { count: tool.usage_count })}</Badge> : null}
          </div>
          <p className="mt-1.5 text-[11.5px] leading-relaxed text-muted">{tool.description}</p>
        </div>
      </div>

      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="mt-2 flex items-center gap-1 text-[11px] text-muted transition-colors hover:text-ink"
      >
        {open ? <ChevronDown size={11} /> : <ChevronRight size={11} />}
        {t('tools.params', { count: fields.length })}
      </button>

      {open ? (
        <div className="mt-1.5 space-y-1">
          {fields.length === 0 ? (
            <p className="text-[11px] text-muted">{t('tools.noParams')}</p>
          ) : (
            fields.map((field) => {
              const schema = (parameters.properties ?? {})[field] as
                | { type?: string; description?: string }
                | undefined;
              return (
                <div key={field} className="flex items-start gap-2 text-[11px]">
                  <code className="shrink-0 rounded bg-canvas px-1 py-0.5 font-mono text-[10.5px] text-ink">
                    {field}
                    {required.includes(field) ? <span className="text-danger">*</span> : null}
                  </code>
                  <span className="text-muted">
                    {schema?.type ? <span className="text-accent">{schema.type} </span> : null}
                    {schema?.description ?? ''}
                  </span>
                </div>
              );
            })
          )}
          {tool.created_by_agent_id ? (
            <p className="pt-1 text-[10.5px] text-muted">
              {t('tools.forgedBy', { id: tool.created_by_agent_id })}
            </p>
          ) : null}
        </div>
      ) : null}
    </Card>
  );
}

/** Toolbox: native primitives plus anything the swarm forged at runtime. */
export function ToolsView() {
  const t = useT();
  const tools = useApp((state) => state.tools);

  const { native, dynamic } = useMemo(() => {
    const ordered = [...tools].sort((a, b) => a.name.localeCompare(b.name));
    return {
      native: ordered.filter((tool) => !tool.is_dynamic),
      dynamic: ordered.filter((tool) => tool.is_dynamic),
    };
  }, [tools]);

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto flex max-w-[1180px] flex-col gap-6 px-6 py-6">
        <header>
          <h1 className="text-[19px] font-semibold tracking-tight text-ink">{t('tools.title')}</h1>
          <p className="mt-1 max-w-[680px] text-[12.5px] leading-relaxed text-muted">{t('tools.subtitle')}</p>
        </header>

        <section>
          <SectionTitle
            title={t('tools.native')}
            hint={t('tools.nativeHint')}
            action={<Badge tone="neutral">{native.length}</Badge>}
          />
          {native.length === 0 ? (
            <EmptyState icon={<Terminal size={20} />} title={t('tools.empty')} />
          ) : (
            <div className="grid grid-cols-1 gap-2.5 lg:grid-cols-2 2xl:grid-cols-3">
              {native.map((tool) => (
                <ToolCard key={tool.name} tool={tool} />
              ))}
            </div>
          )}
        </section>

        <section>
          <SectionTitle
            title={t('tools.forged')}
            hint={t('tools.forgedHint')}
            action={<Badge tone={dynamic.length ? 'accent' : 'neutral'}>{dynamic.length}</Badge>}
          />
          {dynamic.length === 0 ? (
            <Card className="p-5 text-center">
              <div className="flex flex-col items-center gap-2">
                <Hammer size={18} className="text-muted" />
                <p className="text-[12.5px] font-medium text-ink">{t('tools.noneForged')}</p>
                <p className="max-w-md text-[11.5px] leading-relaxed text-muted">
                  {t('tools.noneForgedBody')}
                </p>
              </div>
            </Card>
          ) : (
            <div className="grid grid-cols-1 gap-2.5 lg:grid-cols-2 2xl:grid-cols-3">
              {dynamic.map((tool) => (
                <ToolCard key={tool.name} tool={tool} />
              ))}
            </div>
          )}
        </section>

        <section>
          <SectionTitle title={t('tools.guarantees')} />
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-3">
            {[
              { icon: Code2, titleKey: 'tools.g1.title', bodyKey: 'tools.g1.body' },
              { icon: Terminal, titleKey: 'tools.g2.title', bodyKey: 'tools.g2.body' },
              { icon: Hammer, titleKey: 'tools.g3.title', bodyKey: 'tools.g3.body' },
            ].map((item) => {
              const Icon = item.icon;
              return (
                <Card key={item.titleKey} className="p-3">
                  <div className="flex items-center gap-1.5">
                    <Icon size={12} className="text-accent" />
                    <p className="text-[12px] font-medium text-ink">{t(item.titleKey)}</p>
                  </div>
                  <p className="mt-1 text-[11px] leading-relaxed text-muted">{t(item.bodyKey)}</p>
                </Card>
              );
            })}
          </div>
        </section>
      </div>
    </div>
  );
}
