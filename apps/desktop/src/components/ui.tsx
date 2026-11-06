import {
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from 'react';
import { statusLabel, translate, type Lang } from '../i18n';
import { useApp } from '../store';

export function cx(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(' ');
}

/* ------------------------------------------------------------------ */
/* Buttons                                                             */
/* ------------------------------------------------------------------ */

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
type ButtonSize = 'sm' | 'md';

const BUTTON_VARIANTS: Record<ButtonVariant, string> = {
  primary:
    'bg-accent text-white hover:brightness-110 active:brightness-95 disabled:opacity-45 disabled:hover:brightness-100',
  secondary:
    'bg-surface text-ink border border-line hover:bg-raised active:bg-raised disabled:opacity-45',
  ghost: 'text-muted hover:text-ink hover:bg-raised disabled:opacity-45',
  danger: 'bg-danger text-white hover:brightness-110 disabled:opacity-45',
};

const BUTTON_SIZES: Record<ButtonSize, string> = {
  sm: 'h-7 px-2.5 text-[12px] gap-1.5 rounded-md',
  md: 'h-9 px-3.5 text-[13px] gap-2 rounded-lg',
};

export function Button({
  variant = 'secondary',
  size = 'md',
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant; size?: ButtonSize }) {
  return (
    <button
      type="button"
      className={cx(
        'inline-flex items-center justify-center font-medium transition-colors focus-ring select-none whitespace-nowrap',
        BUTTON_VARIANTS[variant],
        BUTTON_SIZES[size],
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

export function IconButton({
  className,
  label,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={cx(
        'inline-flex h-7 w-7 items-center justify-center rounded-md text-muted transition-colors hover:bg-raised hover:text-ink focus-ring',
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

/* ------------------------------------------------------------------ */
/* Surfaces                                                            */
/* ------------------------------------------------------------------ */

export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return (
    <div className={cx('rounded-xl border border-line bg-surface shadow-card', className)}>{children}</div>
  );
}

export function SectionTitle({
  title,
  hint,
  action,
}: {
  title: string;
  hint?: string;
  action?: ReactNode;
}) {
  return (
    <div className="mb-3 flex items-end justify-between gap-4">
      <div>
        <h2 className="text-[13px] font-semibold uppercase tracking-wide text-muted">{title}</h2>
        {hint ? <p className="mt-1 text-[12.5px] text-muted">{hint}</p> : null}
      </div>
      {action}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Badges                                                              */
/* ------------------------------------------------------------------ */

type BadgeTone = 'neutral' | 'accent' | 'ok' | 'warn' | 'danger';

const BADGE_TONES: Record<BadgeTone, string> = {
  neutral: 'border-line bg-raised text-muted',
  accent: 'border-transparent bg-accentSoft text-accent',
  ok: 'border-transparent bg-ok/10 text-ok',
  warn: 'border-transparent bg-warn/10 text-warn',
  danger: 'border-transparent bg-danger/10 text-danger',
};

export function Badge({
  tone = 'neutral',
  children,
  className,
  title,
}: {
  tone?: BadgeTone;
  children: ReactNode;
  className?: string;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={cx(
        'inline-flex items-center gap-1 rounded-md border px-1.5 py-0.5 text-[11px] font-medium leading-4',
        BADGE_TONES[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}

const STATUS_TONES: Record<string, BadgeTone> = {
  running: 'accent',
  planning: 'warn',
  paused: 'warn',
  queued: 'neutral',
  waiting_dependency: 'warn',
  completed: 'ok',
  failed: 'danger',
  cancelled: 'neutral',
};

export function StatusDot({ status }: { status: string }) {
  const color =
    status === 'running'
      ? 'bg-accent animate-pulseDot'
      : status === 'completed'
        ? 'bg-ok'
        : status === 'failed'
          ? 'bg-danger'
          : status === 'waiting_dependency' || status === 'paused'
            ? 'bg-warn'
            : 'bg-muted/60';
  return <span className={cx('inline-block h-1.5 w-1.5 shrink-0 rounded-full', color)} />;
}

export function StatusBadge({ status }: { status: string }) {
  const lang = useApp((state) => state.lang);
  return (
    <Badge tone={STATUS_TONES[status] ?? 'neutral'}>
      <StatusDot status={status} />
      {statusLabel(lang, status)}
    </Badge>
  );
}

/* ------------------------------------------------------------------ */
/* Form controls                                                       */
/* ------------------------------------------------------------------ */

const CONTROL =
  'w-full rounded-lg border border-line bg-surface px-3 py-2 text-[13px] text-ink placeholder:text-muted/70 focus-ring transition-colors';

export function TextInput({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={cx(CONTROL, 'h-9', className)} {...rest} />;
}

export function TextArea({ className, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className={cx(CONTROL, 'resize-none leading-relaxed', className)} {...rest} />;
}

export function Select({
  className,
  children,
  ...rest
}: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={cx(CONTROL, 'h-8 cursor-pointer pr-7 text-[12.5px]', className)} {...rest}>
      {children}
    </select>
  );
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[12px] font-medium text-muted">{label}</span>
      {children}
      {hint ? <span className="mt-1 block text-[11.5px] text-muted/80">{hint}</span> : null}
    </label>
  );
}

export function Toggle({
  checked,
  onChange,
  label,
  description,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  description?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className="flex w-full items-center justify-between gap-4 rounded-lg border border-line bg-surface px-3 py-2 text-left transition-colors hover:bg-raised focus-ring"
    >
      <span>
        <span className="block text-[13px] font-medium text-ink">{label}</span>
        {description ? <span className="mt-0.5 block text-[11.5px] text-muted">{description}</span> : null}
      </span>
      <span
        className={cx(
          'relative h-5 w-9 shrink-0 rounded-full transition-colors',
          checked ? 'bg-accent' : 'bg-line',
        )}
      >
        <span
          className={cx(
            'absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-all',
            checked ? 'left-[18px]' : 'left-0.5',
          )}
        />
      </span>
    </button>
  );
}

/* ------------------------------------------------------------------ */
/* Feedback                                                            */
/* ------------------------------------------------------------------ */

export function Spinner({ className }: { className?: string }) {
  return (
    <span
      className={cx(
        'inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent',
        className,
      )}
    />
  );
}

export function EmptyState({
  icon,
  title,
  description,
  action,
}: {
  icon?: ReactNode;
  title: string;
  description?: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 rounded-xl border border-dashed border-line bg-surface/60 px-8 py-12 text-center">
      {icon ? <div className="text-muted">{icon}</div> : null}
      <div>
        <p className="text-[14px] font-medium text-ink">{title}</p>
        {description ? <p className="mt-1 max-w-md text-[12.5px] leading-relaxed text-muted">{description}</p> : null}
      </div>
      {action}
    </div>
  );
}

export function ProgressBar({ value, max }: { value: number; max: number }) {
  const pct = max > 0 ? Math.min(100, Math.round((value / max) * 100)) : 8;
  return (
    <div className="h-1 w-full overflow-hidden rounded-full bg-line">
      <div className="h-full rounded-full bg-accent transition-all" style={{ width: `${pct}%` }} />
    </div>
  );
}

/* ------------------------------------------------------------------ */
/* Formatting helpers                                                  */
/* ------------------------------------------------------------------ */

export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return '0s';
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  if (minutes < 60) return `${minutes}m ${rest.toString().padStart(2, '0')}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${(minutes % 60).toString().padStart(2, '0')}m`;
}

export function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return `${value}`;
}

export function formatBytes(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`;
}

export function relativeTime(iso: string, lang: Lang = 'zh'): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return '';
  const diff = Date.now() - then;
  if (diff < 60_000) return translate(lang, 'time.justNow');
  if (diff < 3_600_000) return translate(lang, 'time.minutesAgo', { n: Math.floor(diff / 60_000) });
  if (diff < 86_400_000) return translate(lang, 'time.hoursAgo', { n: Math.floor(diff / 3_600_000) });
  return translate(lang, 'time.daysAgo', { n: Math.floor(diff / 86_400_000) });
}
