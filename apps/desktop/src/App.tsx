import { useEffect } from 'react';
import { listen } from '@tauri-apps/api/event';
import { isDesktop } from './api';
import { useApp } from './store';
import { useT } from './useT';
import type { SystemEvent } from './types';
import { Sidebar } from './components/Sidebar';
import { Toasts } from './components/Toast';
import { InspectorDrawer } from './inspector/InspectorDrawer';
import { TasksView } from './views/TasksView';
import { ModelsView } from './views/ModelsView';
import { ToolsView } from './views/ToolsView';
import { EvolutionView } from './views/EvolutionView';

export default function App() {
  const t = useT();
  const view = useApp((state) => state.view);
  const theme = useApp((state) => state.theme);
  const ready = useApp((state) => state.ready);
  const init = useApp((state) => state.init);
  const handleEvent = useApp((state) => state.handleEvent);

  useEffect(() => {
    document.documentElement.classList.toggle('dark', theme === 'dark');
  }, [theme]);

  useEffect(() => {
    void init();
  }, [init]);

  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void listen<SystemEvent>('reverie-event', (event) => {
      handleEvent(event.payload);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* the event bridge is optional outside the desktop shell */
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [handleEvent]);

  return (
    <div className="flex h-full bg-canvas text-ink">
      <Sidebar />

      <main className="relative min-w-0 flex-1 overflow-hidden">
        {!ready ? (
          <div className="flex h-full items-center justify-center">
            <div className="flex items-center gap-2 text-[12.5px] text-muted">
              <span className="h-3 w-3 animate-spin rounded-full border-2 border-accent border-t-transparent" />
              {t('misc.starting')}
            </div>
          </div>
        ) : (
          <>
            {view === 'tasks' ? <TasksView /> : null}
            {view === 'models' ? <ModelsView /> : null}
            {view === 'tools' ? <ToolsView /> : null}
            {view === 'evolution' ? <EvolutionView /> : null}
          </>
        )}
      </main>

      <InspectorDrawer />
      <Toasts />
    </div>
  );
}
