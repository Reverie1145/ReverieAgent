import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { Cpu, FolderOpen, Languages, ListTodo, Moon, Plus, Sparkles, Sun, Trash2, Wrench } from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import type { ViewKey } from '../types';
import { Badge, Button, IconButton, StatusDot, cx, relativeTime } from './ui';

const NAV: Array<{ key: ViewKey; labelKey: string; icon: typeof ListTodo }> = [
  { key: 'tasks', labelKey: 'nav.tasks', icon: ListTodo },
  { key: 'models', labelKey: 'nav.models', icon: Cpu },
  { key: 'tools', labelKey: 'nav.tools', icon: Wrench },
  { key: 'evolution', labelKey: 'nav.evolution', icon: Sparkles },
];

function folderName(path: string, fallback: string): string {
  if (!path) return fallback;
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** 240px navigation rail. */
export function Sidebar() {
  const t = useT();
  const lang = useApp((state) => state.lang);
  const view = useApp((state) => state.view);
  const setView = useApp((state) => state.setView);
  const sessions = useApp((state) => state.sessions);
  const activeSessionId = useApp((state) => state.activeSessionId);
  const selectSession = useApp((state) => state.selectSession);
  const deleteSession = useApp((state) => state.deleteSession);
  const workspacePath = useApp((state) => state.workspacePath);
  const setWorkspace = useApp((state) => state.setWorkspace);
  const theme = useApp((state) => state.theme);
  const toggleTheme = useApp((state) => state.toggleTheme);
  const toggleLang = useApp((state) => state.toggleLang);
  const info = useApp((state) => state.info);
  const pushToast = useApp((state) => state.pushToast);

  const [picking, setPicking] = useState(false);

  async function pickWorkspace() {
    setPicking(true);
    try {
      const selected = await open({ directory: true, multiple: false, title: t('nav.chooseFolder') });
      if (typeof selected === 'string' && selected) {
        setWorkspace(selected);
        pushToast('success', t('toast.workspaceSet', { path: selected }));
      }
    } catch {
      pushToast('error', t('toast.pickerFailed'));
    } finally {
      setPicking(false);
    }
  }

  return (
    <aside className="flex h-full w-[240px] shrink-0 flex-col border-r border-line bg-surface">
      <div className="flex items-center gap-2 px-3.5 pb-2 pt-4">
        <div className="flex h-7 w-7 items-center justify-center rounded-lg bg-accent">
          <span className="flex items-end gap-[2px]">
            <span className="block h-1.5 w-[3px] rounded-sm bg-white/90" />
            <span className="block h-2.5 w-[3px] rounded-sm bg-white/90" />
            <span className="block h-3.5 w-[3px] rounded-sm bg-accentSoft" />
          </span>
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[13.5px] font-semibold leading-tight tracking-tight">ReverieAgent</p>
          <p className="truncate text-[10.5px] uppercase tracking-wider text-muted">{t('nav.subtitle')}</p>
        </div>
      </div>

      <div className="flex items-center gap-1 px-3.5 pb-2">
        <IconButton label={theme === 'light' ? t('nav.themeLight') : t('nav.themeDark')} onClick={toggleTheme}>
          {theme === 'light' ? <Moon size={13} /> : <Sun size={13} />}
        </IconButton>
        <button
          type="button"
          onClick={toggleLang}
          className="inline-flex h-7 items-center gap-1 rounded-md px-2 text-[11.5px] font-medium text-muted transition-colors hover:bg-raised hover:text-ink focus-ring"
          title={t('nav.lang')}
        >
          <Languages size={13} />
          {t('nav.lang')}
        </button>
      </div>

      <div className="px-3">
        <Button
          variant="primary"
          className="w-full"
          onClick={() => {
            setView('tasks');
            useApp.setState({ activeSessionId: null, snapshot: null });
          }}
        >
          <Plus size={14} />
          {t('nav.newTask')}
        </Button>
      </div>

      <button
        type="button"
        onClick={pickWorkspace}
        disabled={picking}
        title={workspacePath || t('nav.pickProject')}
        className="mx-3 mt-2 flex items-center gap-2 rounded-lg border border-line bg-canvas px-2.5 py-2 text-left transition-colors hover:bg-raised focus-ring"
      >
        <FolderOpen size={13} className="shrink-0 text-muted" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-[12px] font-medium text-ink">
            {folderName(workspacePath, t('nav.chooseFolder'))}
          </span>
          <span className="block truncate text-[10.5px] text-muted">{workspacePath || t('nav.pickProject')}</span>
        </span>
      </button>

      <nav className="mt-3 space-y-0.5 px-2">
        {NAV.map((item) => {
          const Icon = item.icon;
          const active = view === item.key;
          return (
            <button
              key={item.key}
              type="button"
              onClick={() => setView(item.key)}
              className={cx(
                'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-[7px] text-left text-[12.5px] transition-colors focus-ring',
                active ? 'bg-raised font-medium text-ink' : 'text-muted hover:bg-raised/60 hover:text-ink',
              )}
            >
              <Icon size={14} className={active ? 'text-accent' : 'text-muted'} />
              {t(item.labelKey)}
            </button>
          );
        })}
      </nav>

      <div className="mt-4 flex items-center justify-between px-3.5">
        <p className="text-[10.5px] font-semibold uppercase tracking-wider text-muted">{t('nav.sessions')}</p>
        <Badge>{sessions.length}</Badge>
      </div>

      <div className="mt-1.5 flex-1 overflow-y-auto px-2 pb-2">
        {sessions.length === 0 ? (
          <p className="px-2 py-4 text-[11.5px] leading-relaxed text-muted">{t('nav.noTasks')}</p>
        ) : (
          <ul className="space-y-0.5">
            {sessions.map((session) => {
              const active = session.id === activeSessionId;
              return (
                <li key={session.id}>
                  <div
                    className={cx(
                      'group flex items-center gap-2 rounded-lg px-2 py-[7px] transition-colors',
                      active ? 'bg-raised' : 'hover:bg-raised/60',
                    )}
                  >
                    <button
                      type="button"
                      onClick={() => void selectSession(session.id)}
                      className="min-w-0 flex-1 text-left focus-ring"
                    >
                      <span className="flex items-center gap-1.5">
                        <StatusDot status={session.status} />
                        <span
                          className={cx('truncate text-[12px]', active ? 'font-medium text-ink' : 'text-ink/90')}
                        >
                          {session.title || t('nav.newTask')}
                        </span>
                      </span>
                      <span className="mt-0.5 flex items-center gap-1.5 pl-3">
                        <span className="text-[10.5px] text-muted">
                          {relativeTime(session.updated_at, lang)}
                        </span>
                        <span className="text-[10px] text-muted/60">·</span>
                        <span className="truncate text-[10.5px] text-muted">
                          {t(`topo.${session.topology_mode}`)}
                        </span>
                      </span>
                    </button>
                    <button
                      type="button"
                      aria-label={t('common.delete')}
                      onClick={() => void deleteSession(session.id)}
                      className="opacity-0 transition-opacity hover:text-danger group-hover:opacity-100"
                    >
                      <Trash2 size={12} className="text-muted" />
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </div>

      <div className="border-t border-line px-3.5 py-2.5">
        <div className="flex items-center justify-between text-[10.5px] text-muted">
          <span>{info ? `v${info.version}` : 'v0.1.0'}</span>
          <span>
            {info
              ? `${info.model_count} ${t('common.models')} · ${info.tool_count} ${t('common.tools')}`
              : t('nav.init')}
          </span>
        </div>
      </div>
    </aside>
  );
}
