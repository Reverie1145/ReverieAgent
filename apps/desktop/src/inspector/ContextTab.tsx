import { FileText, Image as ImageIcon, Layers } from 'lucide-react';
import { archetypeLabel, thinkLabel } from '../i18n';
import { useApp } from '../store';
import { useT } from '../useT';
import type { AgentDetail } from '../types';
import { Badge, EmptyState, formatBytes } from '../components/ui';

/** Context & attachments mounted into this agent. */
export function ContextTab({ detail }: { detail: AgentDetail | null }) {
  const t = useT();
  const lang = useApp((state) => state.lang);

  if (!detail) {
    return <EmptyState title={t('insp.loadingContext')} />;
  }

  const { agent, attachments, published } = detail;
  const hasAnything = attachments.length > 0 || published.length > 0;

  return (
    <div className="space-y-4">
      <section>
        <p className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted">
          {t('insp.assignment')}
        </p>
        <div className="rounded-lg border border-line bg-surface px-3 py-2.5">
          <p className="text-[11.5px] leading-relaxed text-muted">{agent.task_description}</p>
          <div className="mt-2 flex flex-wrap gap-1">
            <Badge tone="neutral">{archetypeLabel(lang, agent.archetype)}</Badge>
            <Badge tone="neutral">{agent.model_name || 'auto'}</Badge>
            <Badge tone={agent.thinking_level === 'off' ? 'neutral' : 'accent'}>
              {t('think.label', { level: thinkLabel(lang, agent.thinking_level) })}
            </Badge>
            {agent.git_branch ? <Badge tone="accent">{agent.git_branch}</Badge> : null}
          </div>
          <p className="mt-2 truncate text-[10.5px] text-muted" title={agent.workspace_dir}>
            {t('insp.workspace')}: {agent.workspace_dir}
          </p>
        </div>
      </section>

      <section>
        <p className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted">
          {t('insp.mounted', { count: attachments.length })}
        </p>
        {attachments.length === 0 ? (
          <p className="rounded-lg border border-dashed border-line px-3 py-3 text-[11.5px] text-muted">
            {t('insp.noFiles')}
          </p>
        ) : (
          <ul className="space-y-1.5">
            {attachments.map((attachment) => (
              <li
                key={attachment.id}
                className="flex items-start gap-2 rounded-lg border border-line bg-surface px-2.5 py-2"
              >
                {attachment.base64_thumbnail && attachment.mime_type.startsWith('image/') ? (
                  <img
                    src={`data:${attachment.mime_type};base64,${attachment.base64_thumbnail}`}
                    alt={attachment.file_name}
                    className="h-9 w-9 shrink-0 rounded object-cover"
                  />
                ) : (
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded bg-raised text-muted">
                    {attachment.mime_type.startsWith('image/') ? <ImageIcon size={13} /> : <FileText size={13} />}
                  </span>
                )}
                <div className="min-w-0 flex-1">
                  <p className="truncate text-[11.5px] font-medium text-ink">{attachment.file_name}</p>
                  <p className="text-[10px] text-muted">
                    {attachment.mime_type} · {formatBytes(attachment.file_size_bytes)}
                  </p>
                  {attachment.extracted_text_preview ? (
                    <p className="mt-1 line-clamp-2 text-[10.5px] leading-relaxed text-muted">
                      {attachment.extracted_text_preview}
                    </p>
                  ) : null}
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <p className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted">
          {t('insp.published', { count: published.length })}
        </p>
        {published.length === 0 ? (
          <p className="rounded-lg border border-dashed border-line px-3 py-3 text-[11.5px] text-muted">
            {t('insp.notPublished')}
          </p>
        ) : (
          <ul className="space-y-1.5">
            {published.map((entry) => (
              <li key={entry.id} className="rounded-lg border border-line bg-surface px-2.5 py-2">
                <div className="flex items-center gap-1.5">
                  <Badge tone="accent">{entry.topic_tag}</Badge>
                  <span className="text-[10px] text-muted">
                    → {entry.target_agent_ids.includes('*') ? t('common.all') : entry.target_agent_ids.join(', ')}
                  </span>
                </div>
                <p className="mt-1 text-[11.5px] font-medium text-ink">{entry.summary}</p>
                <p className="mt-0.5 line-clamp-3 text-[10.5px] leading-relaxed text-muted">
                  {entry.full_content}
                </p>
              </li>
            ))}
          </ul>
        )}
      </section>

      {!hasAnything ? (
        <EmptyState
          icon={<Layers size={18} />}
          title={t('insp.noShared')}
          description={t('insp.noSharedBody')}
        />
      ) : null}
    </div>
  );
}
