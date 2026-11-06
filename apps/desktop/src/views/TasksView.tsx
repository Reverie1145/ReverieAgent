import { Brain, FolderTree, Globe2, Layers, ShieldCheck } from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import { Composer } from '../components/Composer';
import { TaskStage } from './TaskStage';

const HIGHLIGHTS = [
  { icon: Brain, titleKey: 'empty.h1.title', bodyKey: 'empty.h1.body' },
  { icon: Globe2, titleKey: 'empty.h2.title', bodyKey: 'empty.h2.body' },
  { icon: Layers, titleKey: 'empty.h3.title', bodyKey: 'empty.h3.body' },
  { icon: ShieldCheck, titleKey: 'empty.h4.title', bodyKey: 'empty.h4.body' },
];

function EmptyStage() {
  const t = useT();
  const workspacePath = useApp((state) => state.workspacePath);
  const models = useApp((state) => state.models);
  const providers = useApp((state) => state.providers);
  const setView = useApp((state) => state.setView);

  const needsSetup = providers.length === 0 || models.length === 0;

  return (
    <div className="flex h-full flex-col overflow-y-auto">
      <div className="mx-auto flex w-full max-w-[760px] flex-1 flex-col justify-center px-6 py-10">
        <p className="text-[11px] font-semibold uppercase tracking-[0.16em] text-accent">{t('empty.badge')}</p>
        <h1 className="mt-2 text-[26px] font-semibold leading-tight tracking-tight text-ink">
          {t('empty.headline')}
        </h1>
        <p className="mt-2 max-w-[620px] text-[13px] leading-relaxed text-muted">{t('empty.sub')}</p>

        {needsSetup ? (
          <button
            type="button"
            onClick={() => setView('models')}
            className="mt-4 flex items-center gap-2 rounded-lg border border-warn/40 bg-warn/5 px-3 py-2 text-left transition-colors hover:bg-warn/10"
          >
            <span className="shrink-0 text-[11.5px] font-medium text-warn">{t('empty.setupTitle')}</span>
            <span className="text-[11.5px] text-muted">{t('empty.setupBody')}</span>
          </button>
        ) : null}

        <div className="mt-5">
          <Composer variant="hero" />
          <p className="mt-2 flex items-center gap-1.5 text-[11px] text-muted">
            <FolderTree size={11} />
            {workspacePath ? t('empty.workingIn', { path: workspacePath }) : t('empty.pickWorkspace')}
          </p>
        </div>

        <div className="mt-8 grid grid-cols-1 gap-2 sm:grid-cols-2">
          {HIGHLIGHTS.map((item) => {
            const Icon = item.icon;
            return (
              <div key={item.titleKey} className="rounded-lg border border-line bg-surface p-3">
                <div className="flex items-center gap-1.5">
                  <Icon size={13} className="text-accent" />
                  <p className="text-[12px] font-medium text-ink">{t(item.titleKey)}</p>
                </div>
                <p className="mt-1 text-[11.5px] leading-relaxed text-muted">{t(item.bodyKey)}</p>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** Tasks view: either the empty launcher or the live stage. */
export function TasksView() {
  const t = useT();
  const activeSessionId = useApp((state) => state.activeSessionId);
  const snapshot = useApp((state) => state.snapshot);

  if (activeSessionId && snapshot) return <TaskStage />;
  if (activeSessionId && !snapshot) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-[12.5px] text-muted">{t('stage.loading')}</p>
      </div>
    );
  }
  return <EmptyStage />;
}
