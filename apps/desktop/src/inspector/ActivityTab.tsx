import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, Terminal, Wrench } from 'lucide-react';
import { useApp } from '../store';
import { useT } from '../useT';
import type { AgentDetail, ChatMessage, LiveToolCall } from '../types';
import { Markdown } from '../components/Markdown';
import { Badge, EmptyState, cx } from '../components/ui';

interface TranscriptView {
  reasoning: string;
  output: string;
  tools: LiveToolCall[];
}

/** Rebuild a readable activity view from a persisted transcript. */
export function summarizeTranscript(messages: ChatMessage[]): TranscriptView {
  let reasoning = '';
  let output = '';
  const tools: LiveToolCall[] = [];
  const results = new Map<string, { preview: string; isError: boolean }>();

  for (const message of messages) {
    for (const part of message.parts) {
      if (part.type === 'reasoning') reasoning += part.text;
      if (part.type === 'text' && message.role === 'assistant') output = part.text;
    }
    if (message.role === 'assistant') {
      for (const call of message.tool_calls ?? []) {
        tools.push({ callId: call.id, toolName: call.name, args: call.arguments });
      }
    }
    if (message.role === 'tool' && message.tool_call_id) {
      const text = message.parts.map((part) => (part.type === 'text' ? part.text : '')).join('\n');
      results.set(message.tool_call_id, { preview: text, isError: text.startsWith('[tool_error]') });
    }
  }

  return {
    reasoning,
    output,
    tools: tools.map((call) => {
      const result = results.get(call.callId);
      return result ? { ...call, preview: result.preview, isError: result.isError } : call;
    }),
  };
}

function argsPreview(args: unknown): string {
  if (args === null || args === undefined) return '{}';
  try {
    const text = typeof args === 'string' ? args : JSON.stringify(args);
    return text.length > 180 ? `${text.slice(0, 180)}…` : text;
  } catch {
    return String(args);
  }
}

function ToolRow({ call }: { call: LiveToolCall }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <li className="rounded-lg border border-line bg-canvas">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="flex w-full items-start gap-2 px-2.5 py-2 text-left"
      >
        <Wrench size={11} className={cx('mt-0.5 shrink-0', call.isError ? 'text-danger' : 'text-accent')} />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-1.5">
            <code className="font-mono text-[11.5px] font-medium text-ink">{call.toolName}</code>
            {call.isError ? <Badge tone="danger">error</Badge> : null}
          </span>
          <span className="mt-0.5 block truncate font-mono text-[10.5px] text-muted">
            {argsPreview(call.args)}
          </span>
        </span>
        {open ? (
          <ChevronDown size={11} className="mt-1 text-muted" />
        ) : (
          <ChevronRight size={11} className="mt-1 text-muted" />
        )}
      </button>
      {open ? (
        <div className="border-t border-line px-2.5 py-2">
          <p className="mb-1 text-[10px] font-semibold uppercase tracking-wider text-muted">
            {t('insp.arguments')}
          </p>
          <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded bg-surface p-2 font-mono text-[10.5px] text-muted">
            {argsPreview(call.args)}
          </pre>
          {call.preview ? (
            <>
              <p className="mb-1 mt-2 text-[10px] font-semibold uppercase tracking-wider text-muted">
                {t('insp.result')}
              </p>
              <pre
                className={cx(
                  'max-h-52 overflow-auto whitespace-pre-wrap rounded bg-surface p-2 font-mono text-[10.5px]',
                  call.isError ? 'text-danger' : 'text-muted',
                )}
              >
                {call.preview}
              </pre>
            </>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}

/** Activity & thinking stream for one agent. */
export function ActivityTab({ agentId, detail }: { agentId: string; detail: AgentDetail | null }) {
  const t = useT();
  const live = useApp((state) => state.live[agentId]);
  const [thinkingOpen, setThinkingOpen] = useState(true);

  const transcript = useMemo(() => summarizeTranscript(detail?.messages ?? []), [detail]);

  const reasoning = live?.thinking || transcript.reasoning;
  const output = live?.output || transcript.output;
  const tools = live && live.toolCalls.length ? live.toolCalls : transcript.tools;

  const hasAnything = Boolean(reasoning || output || tools.length);

  if (!hasAnything) {
    return (
      <EmptyState
        icon={<Terminal size={18} />}
        title={t('insp.noActivity')}
        description={t('insp.noActivityBody')}
      />
    );
  }

  return (
    <div className="space-y-3">
      {reasoning ? (
        <div className="rounded-lg border border-line bg-surface">
          <button
            type="button"
            onClick={() => setThinkingOpen((value) => !value)}
            className="flex w-full items-center gap-1.5 px-2.5 py-2 text-left"
          >
            {thinkingOpen ? (
              <ChevronDown size={12} className="text-muted" />
            ) : (
              <ChevronRight size={12} className="text-muted" />
            )}
            <span className="text-[11.5px] font-medium text-ink">{t('stage.thinkingTitle')}</span>
            <Badge tone="accent">{t('insp.live')}</Badge>
          </button>
          {thinkingOpen ? (
            <pre className="max-h-72 overflow-y-auto whitespace-pre-wrap border-t border-line px-2.5 py-2 font-mono text-[11px] leading-relaxed text-muted">
              {reasoning}
            </pre>
          ) : null}
        </div>
      ) : null}

      {tools.length ? (
        <div>
          <p className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted">
            {t('insp.toolActivity', { count: tools.length })}
          </p>
          <ul className="space-y-1.5">
            {tools.map((call) => (
              <ToolRow key={call.callId} call={call} />
            ))}
          </ul>
        </div>
      ) : null}

      {output ? (
        <div>
          <p className="mb-1.5 text-[10.5px] font-semibold uppercase tracking-wider text-muted">
            {t('insp.output')}
          </p>
          <div className="rounded-lg border border-line bg-surface px-3 py-2.5">
            <Markdown>{output}</Markdown>
          </div>
        </div>
      ) : null}
    </div>
  );
}
