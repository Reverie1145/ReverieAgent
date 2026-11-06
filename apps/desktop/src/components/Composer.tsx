import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { FileText, Globe2, Image as ImageIcon, Loader2, Paperclip, Play, X } from 'lucide-react';
import { isDesktop } from '../api';
import { thinkLabel } from '../i18n';
import { useApp } from '../store';
import { useT } from '../useT';
import { THINKING_MODES, type SessionAttachment, type ThinkingMode } from '../types';
import { Badge, Button, Select, cx, formatBytes } from './ui';

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = '';
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function isImage(attachment: SessionAttachment): boolean {
  return attachment.mime_type.startsWith('image/');
}

function AttachmentChip({ attachment, onRemove }: { attachment: SessionAttachment; onRemove: () => void }) {
  const t = useT();
  const [openPreview, setOpenPreview] = useState(false);
  const thumbnail = attachment.base64_thumbnail;

  return (
    <div className="group relative">
      <button
        type="button"
        onClick={() => thumbnail && setOpenPreview((value) => !value)}
        className={cx(
          'flex max-w-[220px] items-center gap-2 rounded-lg border border-line bg-canvas px-2 py-1.5 text-left transition-colors',
          thumbnail ? 'hover:border-accent/40' : 'cursor-default',
        )}
        title={attachment.file_path}
      >
        {thumbnail && isImage(attachment) ? (
          <img
            src={`data:${attachment.mime_type};base64,${thumbnail}`}
            alt={attachment.file_name}
            className="h-7 w-7 shrink-0 rounded object-cover"
          />
        ) : (
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded bg-raised text-muted">
            {isImage(attachment) ? <ImageIcon size={13} /> : <FileText size={13} />}
          </span>
        )}
        <span className="min-w-0">
          <span className="block truncate text-[11.5px] font-medium text-ink">{attachment.file_name}</span>
          <span className="block text-[10px] text-muted">{formatBytes(attachment.file_size_bytes)}</span>
        </span>
      </button>
      <button
        type="button"
        aria-label={t('common.delete')}
        onClick={onRemove}
        className="absolute -right-1.5 -top-1.5 hidden h-4 w-4 items-center justify-center rounded-full border border-line bg-surface text-muted shadow-sm hover:text-danger group-hover:flex"
      >
        <X size={9} />
      </button>
      {openPreview && thumbnail ? (
        <div className="absolute bottom-full left-0 z-30 mb-2 rounded-lg border border-line bg-surface p-1.5 shadow-drawer">
          <img
            src={`data:${attachment.mime_type};base64,${thumbnail}`}
            alt={attachment.file_name}
            className="max-h-64 max-w-xs rounded object-contain"
          />
        </div>
      ) : null}
    </div>
  );
}

/** Multimodal task composer: text, files, web toggle, model + depth, run. */
export function Composer({ variant = 'docked' }: { variant?: 'docked' | 'hero' }) {
  const t = useT();
  const lang = useApp((state) => state.lang);
  const attachments = useApp((state) => state.attachments);
  const addAttachments = useApp((state) => state.addAttachments);
  const addAttachmentBytes = useApp((state) => state.addAttachmentBytes);
  const removeAttachment = useApp((state) => state.removeAttachment);
  const webSearch = useApp((state) => state.webSearch);
  const setWebSearch = useApp((state) => state.setWebSearch);
  const thinkingMode = useApp((state) => state.thinkingMode);
  const setThinkingMode = useApp((state) => state.setThinkingMode);
  const selectedModelId = useApp((state) => state.selectedModelId);
  const setSelectedModel = useApp((state) => state.setSelectedModel);
  const models = useApp((state) => state.models);
  const startTask = useApp((state) => state.startTask);
  const pushToast = useApp((state) => state.pushToast);
  const workspacePath = useApp((state) => state.workspacePath);

  const [objective, setObjective] = useState('');
  const [running, setRunning] = useState(false);
  const [dragActive, setDragActive] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const autoResize = useCallback(() => {
    const node = textareaRef.current;
    if (!node) return;
    node.style.height = 'auto';
    node.style.height = `${Math.min(node.scrollHeight, 220)}px`;
  }, []);

  useEffect(() => {
    autoResize();
  }, [objective, autoResize]);

  useEffect(() => {
    if (!isDesktop) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === 'enter' || payload.type === 'over') {
          setDragActive(true);
        } else if (payload.type === 'drop') {
          setDragActive(false);
          if (payload.paths.length) void addAttachments(payload.paths);
        } else {
          setDragActive(false);
        }
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* drag-drop is unavailable outside the desktop shell */
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [addAttachments]);

  async function pickFiles() {
    try {
      const selected = await open({
        multiple: true,
        title: t('composer.attach'),
        filters: [
          {
            name: 'Images & documents',
            extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'pdf', 'csv', 'md', 'txt', 'json', 'ts', 'tsx', 'rs', 'py', 'sql', 'yaml', 'yml'],
          },
        ],
      });
      const paths = Array.isArray(selected) ? selected : selected ? [selected] : [];
      if (paths.length) await addAttachments(paths);
    } catch {
      pushToast('error', t('toast.pickerFailed'));
    }
  }

  async function handlePaste(event: React.ClipboardEvent<HTMLTextAreaElement>) {
    const items = Array.from(event.clipboardData.items);
    const imageItem = items.find((item) => item.type.startsWith('image/'));
    if (!imageItem) return;
    event.preventDefault();
    const file = imageItem.getAsFile();
    if (!file) return;
    const buffer = await file.arrayBuffer();
    const extension = file.type.split('/')[1] || 'png';
    await addAttachmentBytes(
      file.name && file.name !== 'image.png' ? file.name : `pasted-${Date.now()}.${extension}`,
      file.type,
      arrayBufferToBase64(buffer),
    );
  }

  async function submit() {
    if (running) return;
    setRunning(true);
    try {
      const ok = await startTask(objective);
      if (ok) setObjective('');
    } finally {
      setRunning(false);
    }
  }

  const modelOptions = useMemo(
    () =>
      models.map((model) => ({
        id: model.id,
        label: `${model.model_name}${model.supports_vision ? ` · ${t('composer.vision')}` : ''}`,
      })),
    [models, t],
  );

  const hero = variant === 'hero';

  return (
    <div
      className={cx(
        'relative rounded-xl border bg-surface transition-colors',
        dragActive ? 'border-accent ring-2 ring-accent/20' : 'border-line',
        hero ? 'p-3' : 'p-2.5',
      )}
    >
      {dragActive ? (
        <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-xl bg-accent/5">
          <p className="rounded-md bg-surface px-3 py-1.5 text-[12px] font-medium text-accent shadow-card">
            {t('composer.dropFiles')}
          </p>
        </div>
      ) : null}

      {attachments.length ? (
        <div className="mb-2 flex flex-wrap gap-1.5">
          {attachments.map((attachment) => (
            <AttachmentChip
              key={attachment.id}
              attachment={attachment}
              onRemove={() => removeAttachment(attachment.id)}
            />
          ))}
        </div>
      ) : null}

      <textarea
        ref={textareaRef}
        value={objective}
        onChange={(event) => setObjective(event.target.value)}
        onPaste={handlePaste}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void submit();
          }
        }}
        rows={hero ? 4 : 2}
        placeholder={workspacePath ? t('composer.placeholder') : t('composer.placeholderNoWs')}
        className="w-full resize-none border-0 bg-transparent px-1.5 py-1 text-[13.5px] leading-relaxed text-ink outline-none placeholder:text-muted/70"
      />

      <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
        <Button variant="ghost" size="sm" onClick={() => void pickFiles()}>
          <Paperclip size={13} />
          {t('composer.attach')}
        </Button>

        <button
          type="button"
          onClick={() => setWebSearch(!webSearch)}
          className={cx(
            'inline-flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-[12px] font-medium transition-colors',
            webSearch
              ? 'border-transparent bg-accentSoft text-accent'
              : 'border-line bg-surface text-muted hover:text-ink',
          )}
          title={t('composer.webHint')}
        >
          <Globe2 size={12} />
          {webSearch ? t('composer.webOn') : t('composer.webOff')}
        </button>

        <Select
          value={selectedModelId}
          onChange={(event) => setSelectedModel(event.target.value)}
          className="h-7 max-w-[220px] text-[12px]"
          title={t('composer.autoHint')}
        >
          <option value="">{t('composer.autoModel')}</option>
          {modelOptions.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </Select>

        <Select
          value={thinkingMode}
          onChange={(event) => setThinkingMode(event.target.value as ThinkingMode)}
          className="h-7 w-[132px] text-[12px]"
          title={t('composer.thinkPrefix')}
        >
          {THINKING_MODES.map((mode) => (
            <option key={mode} value={mode}>
              {mode === 'auto'
                ? t('composer.thinkingAuto')
                : `${t('composer.thinkPrefix')} ${thinkLabel(lang, mode)}`}
            </option>
          ))}
        </Select>

        <div className="ml-auto flex items-center gap-2">
          {attachments.length ? <Badge>{t('composer.attached', { count: attachments.length })}</Badge> : null}
          <span className="hidden text-[10.5px] text-muted sm:block">{t('composer.shortcut')}</span>
          <Button variant="primary" size="sm" onClick={() => void submit()} disabled={running || !objective.trim()}>
            {running ? <Loader2 size={13} className="animate-spin" /> : <Play size={13} />}
            {t('composer.run')}
          </Button>
        </div>
      </div>
    </div>
  );
}
