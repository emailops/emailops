import { Fragment, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ChatMessage, ChatTrace, LlmCallTrace, ToolCallTrace } from '@/types';

export function formatLatency(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

function routeLabel(mode: string): string {
  switch (mode) {
    case 'rag_first':
      return 'RAG first';
    case 'tools_first':
      return 'Tools first';
    default:
      return mode;
  }
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      type="button"
      onClick={() => {
        navigator.clipboard.writeText(text).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1200);
        });
      }}
      className="text-[10px] uppercase tracking-wide text-gray-500 hover:text-gray-700"
    >
      {copied ? 'copied' : 'copy'}
    </button>
  );
}

function ToolCallRow({ call }: { call: ToolCallTrace }) {
  const { t } = useTranslation(['chat']);
  const [open, setOpen] = useState(false);
  return (
    <li className="text-xs text-gray-600 py-1">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1 hover:text-gray-900 w-full text-left"
      >
        <svg
          className={`w-3 h-3 transition-transform ${open ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        <span className="font-mono text-gray-800">{call.name}</span>
        <span className="text-gray-400">
          · {formatLatency(call.elapsedMs)} · {call.resultChars} chars
        </span>
      </button>
      {open && (
        <div className="mt-1 ml-4 space-y-1">
          <div>
            <div className="text-gray-400">{t('chat:reasoning.trace.arguments')}</div>
            <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] whitespace-pre-wrap break-all">
              {JSON.stringify(call.arguments, null, 2)}
            </pre>
          </div>
          <div>
            <div className="text-gray-400">{t('chat:reasoning.trace.resultPreview')}</div>
            <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] whitespace-pre-wrap break-all">
              {call.resultPreview}
            </pre>
          </div>
        </div>
      )}
    </li>
  );
}

/** One LLM call (tool round or final stream) — expandable to show the exact
 *  prompt that was sent and the model's response. input/output are only
 *  populated in dev builds (cfg(debug_assertions) on the backend). */
function LlmCallRow({ call }: { call: LlmCallTrace }) {
  const { t } = useTranslation(['chat']);
  const [open, setOpen] = useState(false);
  const hasIO = (call.input && call.input.length > 0) || (call.output && call.output.length > 0);
  const label =
    call.kind === 'tool_round'
      ? `LLM · tool round ${call.round}`
      : call.kind === 'final_stream'
        ? 'LLM · final stream'
        : call.kind;
  const reqParts: string[] = [];
  if (call.kind === 'tool_round' && (call.toolCallsRequested ?? 0) > 0) {
    reqParts.push(`${call.toolCallsRequested} tool call${call.toolCallsRequested === 1 ? '' : 's'}`);
  }
  return (
    <li className="text-xs text-gray-600 py-1">
      <button
        type="button"
        onClick={() => hasIO && setOpen((v) => !v)}
        className={`flex items-baseline gap-1 w-full text-left ${hasIO ? 'hover:text-gray-900' : 'cursor-default'}`}
      >
        {hasIO ? (
          <svg
            className={`w-3 h-3 transition-transform ${open ? 'rotate-90' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
          </svg>
        ) : (
          <span className="inline-block w-3" />
        )}
        <span className="text-gray-800 min-w-[140px]">{label}</span>
        <span className="text-gray-500 tabular-nums">{formatLatency(call.latencyMs)}</span>
        {reqParts.length > 0 && <span className="text-gray-500">· {reqParts.join(' · ')}</span>}
        {call.failed && <span className="text-red-600 italic">{t('chat:reasoning.trace.failed')}</span>}
        {!hasIO && <span className="text-gray-400 italic">{t('chat:reasoning.trace.devOnly')}</span>}
      </button>
      {open && hasIO && (
        <div className="mt-1 ml-4 space-y-1.5">
          {call.input && (
            <div>
              <div className="flex items-baseline gap-2">
                <div className="text-gray-400 uppercase tracking-wide text-[10px]">
                  input · {call.input.length} chars
                </div>
                <CopyButton text={call.input} />
              </div>
              <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] whitespace-pre-wrap break-words max-h-72 overflow-y-auto">
                {call.input}
              </pre>
            </div>
          )}
          {call.output && (
            <div>
              <div className="flex items-baseline gap-2">
                <div className="text-gray-400 uppercase tracking-wide text-[10px]">
                  output · {call.output.length} chars
                </div>
                <CopyButton text={call.output} />
              </div>
              <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] whitespace-pre-wrap break-words max-h-72 overflow-y-auto">
                {call.output}
              </pre>
            </div>
          )}
        </div>
      )}
    </li>
  );
}

interface TimelineStep {
  label: string;
  ms?: number | null;
  detail?: string;
  note?: string;
}

function TimelineSection({ trace }: { trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const steps: TimelineStep[] = [];
  steps.push({ label: 'Route', detail: routeLabel(trace.route.mode) });

  const r = trace.retrieval;
  if (r) {
    if (r.embeddingMs != null) {
      steps.push({ label: 'Embedding', ms: r.embeddingMs });
    }
    if (r.vecSearchMs != null) {
      steps.push({
        label: 'Vector search',
        ms: r.vecSearchMs,
        detail: `${r.vectorHits} hits`,
      });
    } else if (r.vectorFallback) {
      steps.push({ label: 'Vector search', note: 'fallback (skipped)' });
    }
    steps.push({
      label: 'FTS search',
      ms: r.ftsSearchMs,
      detail: `${r.ftsHits} hits`,
    });
    if (r.fetchMs != null && r.fetchMs > 0) {
      steps.push({ label: 'Fetch metadata', ms: r.fetchMs });
    }
    if (r.expansionMs != null && r.expansionMs > 0) {
      steps.push({ label: 'Thread expansion', ms: r.expansionMs });
    }
    steps.push({
      label: 'Retrieval total',
      ms: r.elapsedMs,
      detail: `${r.fusedTopK} fused${
        typeof r.threadDedupCollapsed === 'number' && r.threadDedupCollapsed > 0
          ? ` · dedup ${r.threadDedupCollapsed}`
          : ''
      }`,
    });
  }

  if (trace.toolCalls.length > 0) {
    const total = trace.toolCalls.reduce((acc, c) => acc + c.elapsedMs, 0);
    steps.push({
      label: `Tool calls (${trace.toolCalls.length})`,
      ms: trace.toolLoopMs ?? total,
      detail: trace.toolCalls.map((c) => c.name).join(', '),
    });
  } else if (trace.toolLoopMs != null && trace.toolLoopMs > 0) {
    steps.push({ label: 'Tool loop', ms: trace.toolLoopMs, note: 'no calls' });
  }

  if (trace.llmStreamingMs != null) {
    steps.push({ label: 'LLM streaming', ms: trace.llmStreamingMs });
  }

  steps.push({ label: 'Total', ms: trace.totalElapsedMs });

  return (
    <div>
      <div className="text-gray-500 uppercase tracking-wide text-[10px] mb-0.5">
        {t('chat:reasoning.trace.timeline')}
      </div>
      <ul className="space-y-0.5">
        {steps.map((s) => (
          <li key={s.label} className="flex items-baseline gap-2">
            <span className="inline-block w-1 h-1 rounded-full bg-gray-400 mt-1" />
            <span className="text-gray-800 min-w-[120px]">{s.label}</span>
            {s.ms != null && <span className="text-gray-500 tabular-nums">{formatLatency(s.ms)}</span>}
            {s.detail && <span className="text-gray-500 truncate">· {s.detail}</span>}
            {s.note && <span className="text-amber-600 italic">· {s.note}</span>}
          </li>
        ))}
      </ul>
      {r?.categories && r.categories.length > 0 && (
        <div className="text-gray-500 mt-0.5 ml-3">categories: {r.categories.join(', ')}</div>
      )}
      {r?.vectorFallback && !steps.some((s) => s.note === 'fallback (skipped)') && (
        <div className="text-amber-600 mt-0.5 ml-3">{t('chat:reasoning.trace.vectorFallback')}</div>
      )}
    </div>
  );
}

export function ReasoningSection({ trace }: { trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const [isOpen, setIsOpen] = useState(false);

  return (
    <div className="mt-2 pt-2 border-t border-gray-200">
      <button
        type="button"
        onClick={() => setIsOpen((v) => !v)}
        className="flex items-center gap-1 text-xs text-gray-500 hover:text-gray-700 transition-colors"
      >
        <svg
          className={`w-3 h-3 transition-transform ${isOpen ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        {t('chat:reasoning.show')}
        <span className="text-gray-400">
          · {routeLabel(trace.route.mode)} · {formatLatency(trace.totalElapsedMs)}
        </span>
      </button>

      {isOpen && (
        <div className="mt-2 space-y-2 text-xs text-gray-700">
          {/* Route */}
          <div>
            <div className="text-gray-500 uppercase tracking-wide text-[10px] mb-0.5">
              {t('chat:reasoning.trace.route')}
            </div>
            <div>
              <span className="font-medium">{routeLabel(trace.route.mode)}</span>
              <span className="text-gray-500"> · {trace.route.classifier}</span>
            </div>
            <div className="text-gray-500">{trace.route.reason}</div>
            {trace.route.matchedKeywords.length > 0 && (
              <div className="mt-0.5 flex flex-wrap gap-1">
                {trace.route.matchedKeywords.map((kw) => (
                  <span
                    key={kw}
                    className="px-1.5 py-0.5 rounded bg-gray-100 border border-gray-200 font-mono text-[11px]"
                  >
                    {kw}
                  </span>
                ))}
              </div>
            )}
          </div>

          {/* Step-by-step timeline */}
          <TimelineSection trace={trace} />

          {/* Workflow — every LLM call in order, expandable to show prompt
              + response (input/output only populated in dev builds). The
              chronological list is: tool_round 0 → tool_round 1 → … →
              final_stream, so it doubles as the workflow timeline. */}
          {(trace.llmCalls?.length ?? 0) > 0 && (
            <div>
              <div className="text-gray-500 uppercase tracking-wide text-[10px] mb-0.5">
                {t('chat:reasoning.trace.workflow')}
              </div>
              <ul className="space-y-0.5">
                {trace.llmCalls?.map((c) => (
                  <LlmCallRow key={`${c.kind}-${c.round}`} call={c} />
                ))}
              </ul>
            </div>
          )}

          {/* Tool calls (expandable detail, separate from the timeline above) */}
          {trace.toolCalls.length > 0 && (
            <div>
              <div className="text-gray-500 uppercase tracking-wide text-[10px] mb-0.5">
                {t('chat:reasoning.trace.toolCallDetails')}
              </div>
              <ul className="space-y-0.5">
                {trace.toolCalls.map((c, i) => (
                  // Trace data is immutable after render, so index is stable.
                  // biome-ignore lint/suspicious/noArrayIndexKey: tool calls may repeat (same name twice) and the list is never reordered
                  <ToolCallRow key={i} call={c} />
                ))}
              </ul>
            </div>
          )}

          {/* Model */}
          <div className="text-gray-500 text-[11px]">
            model <span className="font-mono">{trace.model}</span>
          </div>
        </div>
      )}
    </div>
  );
}

export function StatsFooter({ message }: { message: ChatMessage }) {
  const parts: string[] = [];
  if (message.model) parts.push(message.model);
  if (message.tokenCount != null) parts.push(`${message.tokenCount} tokens`);
  if (message.latencyMs != null) parts.push(formatLatency(message.latencyMs));

  if (parts.length === 0) return null;

  return (
    <div className="mt-2 pt-1.5 border-t border-gray-200 text-[11px] text-gray-400 flex items-center gap-1.5 flex-wrap">
      {parts.map((p, i) => (
        <Fragment key={p}>
          {i > 0 && <span className="text-gray-300">·</span>}
          <span>{p}</span>
        </Fragment>
      ))}
    </div>
  );
}
