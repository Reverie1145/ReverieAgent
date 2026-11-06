import { AlertCircle, CheckCircle2, Info, X } from 'lucide-react';
import { useApp } from '../store';
import { cx } from './ui';

const TONE = {
  info: { icon: Info, className: 'border-line bg-surface text-ink' },
  success: { icon: CheckCircle2, className: 'border-ok/30 bg-ok/5 text-ink' },
  error: { icon: AlertCircle, className: 'border-danger/30 bg-danger/5 text-ink' },
} as const;

/** Bottom-right stack of transient notifications. */
export function Toasts() {
  const toasts = useApp((state) => state.toasts);
  const dismiss = useApp((state) => state.dismissToast);

  if (!toasts.length) return null;

  return (
    <div className="pointer-events-none fixed bottom-5 right-5 z-[60] flex w-[380px] flex-col gap-2">
      {toasts.map((toast) => {
        const tone = TONE[toast.level];
        const Icon = tone.icon;
        return (
          <div
            key={toast.id}
            className={cx(
              'pointer-events-auto flex items-start gap-2.5 rounded-lg border px-3.5 py-2.5 shadow-card animate-fade-in',
              tone.className,
            )}
          >
            <Icon
              size={15}
              className={cx(
                'mt-0.5 shrink-0',
                toast.level === 'error' ? 'text-danger' : toast.level === 'success' ? 'text-ok' : 'text-accent',
              )}
            />
            <p className="flex-1 text-[12.5px] leading-relaxed">{toast.message}</p>
            <button
              type="button"
              aria-label="Dismiss"
              onClick={() => dismiss(toast.id)}
              className="mt-0.5 text-muted transition-colors hover:text-ink"
            >
              <X size={13} />
            </button>
          </div>
        );
      })}
    </div>
  );
}
