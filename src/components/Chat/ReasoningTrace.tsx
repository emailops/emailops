import { Fragment, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  buildWaterfall,
  formatLatency,
  groupWorkflow,
  tokensPerSecond,
  type WaterfallPhase,
} from '@/lib/reasoningTrace';
import type { ChatMessage, ChatTrace, LlmCallTrace, RouteMode, ToolCallTrace } from '@/types';

/** Re-exported so existing callers keep a single import site for latency formatting. */
export { formatLatency };

/** Tailwind bar color per waterfall phase group. */
const PHASE_COLOR: Record<WaterfallPhase['group'], string> = {
  retrieval: 'bg-sky-500',
  llm: 'bg-primary-500',
  tools: 'bg-amber-500',
};

function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation(['chat']);
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
      className="text-[11px] uppercase tracking-wide text-gray-600 hover:text-gray-900"
    >
      {copied ? t('chat:reasoning.trace.copied') : t('chat:reasoning.trace.copy')}
    </button>
  );
}

function ToolCallRow({ call }: { call: ToolCallTrace }) {
  const { t } = useTranslation(['chat']);
  const [open, setOpen] = useState(false);
  return (
    <li className="text-[13px] text-gray-700 py-1">
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
        <span className="font-mono text-gray-900">
          <span className="text-gray-500">{t('chat:reasoning.trace.tool')}</span> {call.name}
        </span>
        <span className="text-gray-600 tabular-nums">
          · {formatLatency(call.elapsedMs)} · {t('chat:reasoning.trace.chars', { n: call.resultChars })}
        </span>
      </button>
      {open && (
        <div className="mt-1 ml-4 space-y-1">
          <div>
            <div className="text-gray-600">{t('chat:reasoning.trace.arguments')}</div>
            <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] text-gray-800 whitespace-pre-wrap break-all">
              {JSON.stringify(call.arguments, null, 2)}
            </pre>
          </div>
          <div>
            <div className="text-gray-600">{t('chat:reasoning.trace.resultPreview')}</div>
            <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] text-gray-800 whitespace-pre-wrap break-all">
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
      ? t('chat:reasoning.trace.phase.toolRound', { n: call.round })
      : call.kind === 'final_stream'
        ? t('chat:reasoning.trace.phase.finalStream')
        : t('chat:reasoning.trace.phase.llmCall');
  const requested = call.kind === 'tool_round' ? (call.toolCallsRequested ?? 0) : 0;
  return (
    <li className="text-[13px] text-gray-700 py-1">
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
        <span className="text-gray-900 min-w-[140px]">{label}</span>
        <span className="text-gray-700 tabular-nums">{formatLatency(call.latencyMs)}</span>
        {requested > 0 && (
          <span className="text-gray-700">· {t('chat:reasoning.trace.toolCallsRequested', { n: requested })}</span>
        )}
        {call.failed && <span className="text-red-600 italic">{t('chat:reasoning.trace.failed')}</span>}
        {!hasIO && <span className="text-gray-500 italic">{t('chat:reasoning.trace.devOnly')}</span>}
      </button>
      {open && hasIO && (
        <div className="mt-1 ml-4 space-y-1.5">
          {call.input && (
            <div>
              <div className="flex items-baseline gap-2">
                <div className="text-gray-600 uppercase tracking-wide text-[11px]">
                  {t('chat:reasoning.trace.input')} · {t('chat:reasoning.trace.chars', { n: call.input.length })}
                </div>
                <CopyButton text={call.input} />
              </div>
              <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] text-gray-800 whitespace-pre-wrap break-words max-h-72 overflow-y-auto">
                {call.input}
              </pre>
            </div>
          )}
          {call.output && (
            <div>
              <div className="flex items-baseline gap-2">
                <div className="text-gray-600 uppercase tracking-wide text-[11px]">
                  {t('chat:reasoning.trace.output')} · {t('chat:reasoning.trace.chars', { n: call.output.length })}
                </div>
                <CopyButton text={call.output} />
              </div>
              <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] text-gray-800 whitespace-pre-wrap break-words max-h-72 overflow-y-auto">
                {call.output}
              </pre>
            </div>
          )}
        </div>
      )}
    </li>
  );
}

/** Proportional latency waterfall — one bar per phase that consumed wall-clock
 *  time. Replaces the old flat step list; the per-step retrieval timings live
 *  in `RetrievalDetail` below. */
function LatencyWaterfall({ trace }: { trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const phases = buildWaterfall(trace);
  if (phases.length === 0) {
    return null;
  }
  return (
    <div>
      <div className="text-gray-600 uppercase tracking-wide text-xs mb-1">
        {t('chat:reasoning.trace.timeline')} · {formatLatency(trace.totalElapsedMs)}
      </div>
      <ul className="space-y-1">
        {phases.map((p) => (
          <li key={p.id} className="flex items-center gap-2">
            <span className={`min-w-[160px] truncate ${p.label ? 'font-mono text-gray-700' : 'text-gray-800'}`}>
              {p.label
                ? `tool: ${p.label}`
                : p.labelKey && t(`chat:reasoning.trace.phase.${p.labelKey}` as const, p.labelParams)}
            </span>
            <span className="flex-1 h-2 rounded bg-gray-100 overflow-hidden">
              <span
                className={`block h-full rounded ${p.failed ? 'bg-red-500' : PHASE_COLOR[p.group]}`}
                style={{ width: `${Math.max(p.fraction * 100, 2)}%` }}
              />
            </span>
            <span className="text-gray-700 tabular-nums min-w-[52px] text-right">{formatLatency(p.ms)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Per-step retrieval timings + counts — the granular detail a developer wants
 *  when the retrieval bar in the waterfall looks slow. */
function RetrievalDetail({ trace }: { trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const r = trace.retrieval;
  if (!r) {
    return null;
  }

  const steps: { key: string; label: string; ms?: number | null; detail?: string; note?: string }[] = [];
  if (r.embeddingMs != null) {
    steps.push({ key: 'embedding', label: t('chat:reasoning.trace.step.embedding'), ms: r.embeddingMs });
  }
  if (r.vecSearchMs != null) {
    steps.push({
      key: 'vectorSearch',
      label: t('chat:reasoning.trace.step.vectorSearch'),
      ms: r.vecSearchMs,
      detail: t('chat:reasoning.trace.hits', { n: r.vectorHits }),
    });
  } else if (r.vectorFallback) {
    steps.push({
      key: 'vectorSearch',
      label: t('chat:reasoning.trace.step.vectorSearch'),
      note: t('chat:reasoning.trace.fallbackSkipped'),
    });
  }
  steps.push({
    key: 'ftsSearch',
    label: t('chat:reasoning.trace.step.ftsSearch'),
    ms: r.ftsSearchMs,
    detail: t('chat:reasoning.trace.hits', { n: r.ftsHits }),
  });
  if (r.fetchMs != null && r.fetchMs > 0) {
    steps.push({ key: 'fetchMetadata', label: t('chat:reasoning.trace.step.fetchMetadata'), ms: r.fetchMs });
  }
  if (r.expansionMs != null && r.expansionMs > 0) {
    steps.push({ key: 'threadExpansion', label: t('chat:reasoning.trace.step.threadExpansion'), ms: r.expansionMs });
  }

  const dedup =
    typeof r.threadDedupCollapsed === 'number' && r.threadDedupCollapsed > 0
      ? ` · ${t('chat:reasoning.trace.dedup', { n: r.threadDedupCollapsed })}`
      : '';

  return (
    <div>
      <div className="text-gray-600 uppercase tracking-wide text-xs mb-0.5">
        {t('chat:reasoning.trace.retrievalDetail')}
      </div>
      <ul className="space-y-0.5">
        {steps.map((s) => (
          <li key={s.key} className="flex items-baseline gap-2">
            <span className="inline-block w-1 h-1 rounded-full bg-gray-400 mt-1" />
            <span className="text-gray-800 min-w-[120px]">{s.label}</span>
            {s.ms != null && <span className="text-gray-700 tabular-nums">{formatLatency(s.ms)}</span>}
            {s.detail && <span className="text-gray-600">· {s.detail}</span>}
            {s.note && <span className="text-amber-700 italic">· {s.note}</span>}
          </li>
        ))}
      </ul>
      <div className="text-gray-700 mt-0.5 ml-3">
        {t('chat:reasoning.trace.fused', { n: r.fusedTopK })}
        {dedup}
      </div>
      {r.categories && r.categories.length > 0 && (
        <div className="text-gray-700 mt-0.5 ml-3">
          {t('chat:reasoning.trace.categories')}: {r.categories.join(', ')}
        </div>
      )}
    </div>
  );
}

export function ReasoningSection({ trace }: { trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const [isOpen, setIsOpen] = useState(false);

  const routeMode = (mode: RouteMode | string): string =>
    mode === 'rag_first' || mode === 'tools_first' ? t(`chat:reasoning.trace.routeMode.${mode}` as const) : mode;

  const { items, unattributed } = groupWorkflow(trace.llmCalls ?? [], trace.toolCalls);

  return (
    <div className="mt-2 pt-2 border-t border-gray-200">
      <button
        type="button"
        onClick={() => setIsOpen((v) => !v)}
        className="flex items-center gap-1 text-[13px] text-gray-600 hover:text-gray-900 transition-colors"
      >
        <svg
          className={`w-3 h-3 transition-transform ${isOpen ? 'rotate-90' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
        </svg>
        {isOpen ? t('chat:reasoning.hide') : t('chat:reasoning.show')}
        <span className="text-gray-500">
          · {routeMode(trace.route.mode)} · {formatLatency(trace.totalElapsedMs)}
        </span>
      </button>

      {isOpen && (
        <div className="mt-2 space-y-3 text-[13px] text-gray-800">
          {/* Route — mode, classifier, reason, matched keywords */}
          <div>
            <div className="text-gray-600 uppercase tracking-wide text-xs mb-0.5">
              {t('chat:reasoning.trace.route')}
            </div>
            <div>
              <span className="font-medium text-gray-900">{routeMode(trace.route.mode)}</span>
              <span className="text-gray-600"> · {trace.route.classifier}</span>
            </div>
            {trace.route.reason && <div className="text-gray-700">{trace.route.reason}</div>}
            {trace.route.matchedKeywords.length > 0 && (
              <div className="mt-0.5 flex flex-wrap gap-1">
                {trace.route.matchedKeywords.map((kw) => (
                  <span
                    key={kw}
                    className="px-1.5 py-0.5 rounded bg-gray-100 border border-gray-200 font-mono text-[11px] text-gray-800"
                  >
                    {kw}
                  </span>
                ))}
              </div>
            )}
          </div>

          {/* Proportional latency waterfall */}
          <LatencyWaterfall trace={trace} />

          {/* Per-step retrieval breakdown */}
          <RetrievalDetail trace={trace} />

          {/* Workflow — every LLM call in order, with the tool calls each
              round triggered nested beneath it so the full flow reads top to
              bottom: tool_round 0 → its tool calls → tool_round 1 → … →
              final_stream. LLM rows expand to the exact prompt + response
              (input/output only populated in dev builds); tool rows expand to
              arguments + result preview. Tool calls that couldn't be tied to a
              round (no llmCalls records, or request counts that didn't add up)
              are listed at the end so nothing is hidden. */}
          {(items.length > 0 || unattributed.length > 0) && (
            <div>
              <div className="text-gray-600 uppercase tracking-wide text-xs mb-0.5">
                {t('chat:reasoning.trace.workflow')}
              </div>
              <ul className="space-y-0.5">
                {items.map(({ call, tools }) => (
                  <Fragment key={`${call.kind}-${call.round}`}>
                    <LlmCallRow call={call} />
                    {tools.length > 0 && (
                      <li className="ml-5 border-l border-gray-200 pl-2">
                        <ul className="space-y-0.5">
                          {tools.map((c, i) => (
                            // biome-ignore lint/suspicious/noArrayIndexKey: tool calls may repeat (same name twice) and the list is never reordered
                            <ToolCallRow key={i} call={c} />
                          ))}
                        </ul>
                      </li>
                    )}
                  </Fragment>
                ))}
                {unattributed.map((c, i) => (
                  // biome-ignore lint/suspicious/noArrayIndexKey: tool calls may repeat (same name twice) and the list is never reordered
                  <ToolCallRow key={`unattributed-${i}`} call={c} />
                ))}
              </ul>
            </div>
          )}

          {/* Model */}
          <div className="text-gray-600 text-xs">
            {t('chat:reasoning.trace.model')} <span className="font-mono text-gray-800">{trace.model}</span>
          </div>
        </div>
      )}
    </div>
  );
}

export function StatsFooter({ message }: { message: ChatMessage }) {
  const { t } = useTranslation(['chat']);
  const parts: string[] = [];
  if (message.model) parts.push(message.model);
  if (message.tokenCount != null) parts.push(t('chat:reasoning.trace.tokens', { n: message.tokenCount }));
  if (message.latencyMs != null) parts.push(formatLatency(message.latencyMs));
  const rate = tokensPerSecond(message.tokenCount, message.latencyMs);
  if (rate > 0) parts.push(t('chat:reasoning.trace.throughput', { rate: rate.toFixed(1) }));

  if (parts.length === 0) return null;

  return (
    <div className="mt-2 pt-1.5 border-t border-gray-200 text-xs text-gray-500 flex items-center gap-1.5 flex-wrap">
      {parts.map((p, i) => (
        <Fragment key={p}>
          {i > 0 && <span className="text-gray-400">·</span>}
          <span>{p}</span>
        </Fragment>
      ))}
    </div>
  );
}
