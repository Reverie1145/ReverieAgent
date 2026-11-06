import { useMemo } from 'react';
import { GitBranch, FolderTree, FileDiff } from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import type { AgentDetail } from '../types';
import { Badge, EmptyState, cx } from '../components/ui';

function DiffLines({ diff }: { diff: string }) {
  const lines = diff.split('\n');
  return (
    <pre className="overflow-x-auto rounded-lg border border-line bg-canvas p-2 font-mono text-[10.5px] leading-[1.55]">
      {lines.map((line, index) => {
        const isAdd = line.startsWith('+') && !line.startsWith('+++');
        const isDel = line.startsWith('-') && !line.startsWith('---');
        const isHunk = line.startsWith('@@');
        const isHeader = line.startsWith('+++') || line.startsWith('---') || line.startsWith('diff ');
        return (
          <div
            key={`${index}-${line.slice(0, 8)}`}
            className={cx(
              'whitespace-pre',
              isAdd && 'bg-ok/10 text-ok',
              isDel && 'bg-danger/10 text-danger',
              isHunk && 'text-accent',
              isHeader && 'font-semibold text-muted',
              !isAdd && !isDel && !isHunk && !isHeader && 'text-muted',
            )}
          >
            {line || ' '}
          </div>
        );
      })}
    </pre>
  );
}

/** File changes produced by one agent. */
export function DiffTab({ agentId, detail }: { agentId: string; detail: AgentDetail | null }) {
  const t = useT();
  const diffs = useApp((state) => state.diffs);

  const mine = useMemo(() => {
    const seen = new Map<string, string>();
    for (const entry of diffs) {
      if (entry.agentId !== agentId) continue;
      // Keep the latest diff per file: it represents the current state.
      seen.set(entry.filePath, entry.diff);
    }
    return [...seen.entries()].map(([filePath, diff]) => ({ filePath, diff }));
  }, [agentId, diffs]);

  return (
    <div className="space-y-3">
      <div className="rounded-lg border border-line bg-surface px-2.5 py-2">
        <div className="flex items-center gap-1.5 text-[11px] text-muted">
          <FolderTree size={11} />
          <span className="truncate" title={detail?.agent.workspace_dir}>
            {detail?.agent.workspace_dir ?? '—'}
          </span>
        </div>
        <div className="mt-1 flex items-center gap-1.5">
          <GitBranch size={11} className="text-muted" />
          {detail?.agent.git_branch ? (
            <Badge tone="accent">{detail.agent.git_branch}</Badge>
          ) : (
            <span className="text-[11px] text-muted">{t('insp.noBranch')}</span>
          )}
        </div>
      </div>

      {mine.length === 0 ? (
        <EmptyState
          icon={<FileDiff size={18} />}
          title={t('insp.noChanges')}
          description={t('insp.noChangesBody')}
        />
      ) : (
        <div className="space-y-2.5">
          <p className="text-[10.5px] font-semibold uppercase tracking-wider text-muted">
            {t('insp.changedFiles', { count: mine.length })}
          </p>
          {mine.map((entry) => (
            <div key={entry.filePath}>
              <p className="mb-1 flex items-center gap-1.5 font-mono text-[11px] text-ink">
                <FileDiff size={11} className="text-accent" />
                {entry.filePath}
              </p>
              <DiffLines diff={entry.diff} />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
