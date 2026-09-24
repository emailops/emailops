import { Fragment, type ReactNode, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { formatLatency, tokensPerSecond } from '@/lib/reasoningTrace';
import { buildFlow, type FlowPhase, type FlowStep } from '@/lib/traceFlow';
import type { ChatMessage, ChatTrace } from '@/types';

/** Re-exported so existing callers keep a single import site for latency formatting. */
export { formatLatency };

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

/** Who decided the route, in words: the backend's classifier ids
 *  ("heuristic", "planner", "forced"…) mean nothing to a reader. */
const ROUTE_CLASSIFIERS = ['heuristic', 'heuristic_followup', 'planner', 'forced', 'ambient', 'llm'] as const;

/** Phase tags stay technical and untranslated, like the `tool:` label. */
const PHASE_TAG: Record<FlowPhase, string> = {
  gather: 'GATHER',
  map: 'MAP',
  condense: 'CONDENSE',
  reduce: 'REDUCE',
};

type Section = 'prompt' | 'output' | 'details';

function Pre({ text }: { text: string }) {
  return (
    <pre className="mt-0.5 p-1.5 rounded bg-gray-50 border border-gray-200 text-[11px] text-gray-800 whitespace-pre-wrap break-words max-h-72 overflow-y-auto">
      {text}
    </pre>
  );
}

/** The title of a step: what ran, in words, plus its short result. */
function StepTitle({ step, trace }: { step: FlowStep; trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  let title: ReactNode;
  switch (step.kind) {
    case 'router': {
      const c = trace.route.classifier;
      const who = (ROUTE_CLASSIFIERS as readonly string[]).includes(c)
        ? t(`chat:reasoning.trace.classifier.${c as (typeof ROUTE_CLASSIFIERS)[number]}`)
        : c;
      title = (
        <>
          {t('chat:reasoning.flow.kind.router')} <span className="text-gray-600">({who})</span>
        </>
      );
      break;
    }
    case 'llmCall':
      title = step.ordinal
        ? t('chat:reasoning.flow.kind.llmCallN', { n: step.ordinal.n, total: step.ordinal.total })
        : t('chat:reasoning.flow.kind.llmCall');
      break;
    case 'llmRound':
      title = t('chat:reasoning.flow.kind.llmRound', { n: step.round ?? 0 });
      break;
    default:
      title = t(`chat:reasoning.flow.kind.${step.kind}` as const);
  }
  return (
    <>
      {step.phase && (
        <span className="font-mono text-[11px] text-primary-700 bg-primary-50 border border-primary-100 rounded px-1">
          [{PHASE_TAG[step.phase]}]
        </span>
      )}
      <span className="text-gray-900">{title}</span>
      {step.summary && (
        <span className="font-mono text-[12px] text-gray-700 break-all">
          {step.kind === 'router' ? `${t('chat:reasoning.flow.keywords')}: ${step.summary}` : step.summary}
        </span>
      )}
      {step.findings != null && (
        <span className="text-gray-600">→ {t('chat:reasoning.flow.findings', { n: step.findings })}</span>
      )}
      {step.failed && <span className="text-red-600 italic">{t('chat:reasoning.trace.failed')}</span>}
    </>
  );
}

function FlowStepRow({ step, index, trace }: { step: FlowStep; index: number; trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const [open, setOpen] = useState<Record<Section, boolean>>({ prompt: false, output: false, details: false });
  const hasDetails = step.details.length > 0 || step.kind === 'retrieval';
  const sections: Section[] = [
    ...(step.prompt ? (['prompt'] as const) : []),
    ...(step.output ? (['output'] as const) : []),
    ...(hasDetails ? (['details'] as const) : []),
  ];
  return (
    <li data-testid="trace-step" className="py-1 text-[13px] text-gray-700">
      <div className="flex items-baseline gap-1.5 flex-wrap">
        <span className="text-gray-500 tabular-nums w-5 shrink-0 text-right">{index + 1}.</span>
        <StepTitle step={step} trace={trace} />
      </div>
      {sections.length > 0 && (
        <div className="ml-6 mt-0.5 flex gap-3 text-[11px] uppercase tracking-wide">
          {sections.map((section) => (
            <button
              key={section}
              type="button"
              data-section={section}
              aria-expanded={open[section]}
              onClick={() => setOpen((o) => ({ ...o, [section]: !o[section] }))}
              className={`flex items-center gap-0.5 ${open[section] ? 'text-gray-900' : 'text-gray-500 hover:text-gray-800'}`}
            >
              <svg
                className={`w-2.5 h-2.5 transition-transform ${open[section] ? 'rotate-90' : ''}`}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 5l7 7-7 7" />
              </svg>
              {t(`chat:reasoning.flow.section.${section}` as const)}
            </button>
          ))}
        </div>
      )}
      <div className="ml-6 space-y-1.5">
        {open.prompt && step.prompt && (
          <div>
            <div className="flex items-baseline gap-2 text-[11px] text-gray-600">
              {t('chat:reasoning.trace.chars', { n: step.prompt.length })}
              <CopyButton text={step.prompt} />
            </div>
            <Pre text={step.prompt} />
          </div>
        )}
        {open.output && step.output && (
          <div>
            <div className="flex items-baseline gap-2 text-[11px] text-gray-600">
              {t('chat:reasoning.trace.chars', { n: step.output.length })}
              <CopyButton text={step.output} />
            </div>
            <Pre text={step.output} />
          </div>
        )}
        {open.details && (
          <div className="text-[12px] text-gray-700">
            {step.details.length > 0 && (
              <ul className="space-y-0.5">
                {step.details.map((d) => (
                  <li key={d.label} className="flex gap-2">
                    <span className="text-gray-500 min-w-[90px]">
                      {t(`chat:reasoning.flow.detail.${d.label}` as const)}
                    </span>
                    <span className="tabular-nums break-all">{d.value}</span>
                  </li>
                ))}
              </ul>
            )}
            {step.kind === 'retrieval' && <RetrievalBreakdown trace={trace} />}
          </div>
        )}
      </div>
    </li>
  );
}

/** A research turn's summary, above its steps. */
function ResearchHeader({ trace }: { trace: ChatTrace }) {
  const { t } = useTranslation(['chat']);
  const r = trace.research;
  if (!r) return null;
  return (
    <div className="mb-1 text-[13px]">
      <span className="font-medium text-primary-700">{t('chat:reasoning.flow.researchOn')}</span>
      <span className="text-gray-700">
        {' · '}
        {t('chat:reasoning.flow.researchRead', {
          read: r.emailsAnalyzed,
          planned: r.plannedEmails,
          batches: r.batches,
        })}
        {' · '}
        {t('chat:reasoning.flow.researchFindings', { findings: r.findings, relevant: r.relevantEmails })}
      </span>
      {r.stopped && <span className="text-amber-700"> · {t('chat:reasoning.flow.researchStopped')}</span>}
      {r.failedBatches > 0 && (
        <span className="text-amber-700"> · {t('chat:reasoning.trace.researchFailed', { n: r.failedBatches })}</span>
      )}
    </div>
  );
}

/** Per-step retrieval timings + counts — the granular detail a developer wants
 *  when the retrieval step in the flow looks slow. */
function RetrievalBreakdown({ trace }: { trace: ChatTrace }) {
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
    <div className="py-0.5">
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
  const flow = buildFlow(trace);
  const mode =
    trace.research != null
      ? t('chat:reasoning.flow.researchOn')
      : trace.route.mode === 'rag_first' || trace.route.mode === 'tools_first'
        ? t(`chat:reasoning.trace.routeMode.${trace.route.mode}` as const)
        : trace.route.mode;

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
          · {mode} · {formatLatency(trace.totalElapsedMs)}
        </span>
      </button>

      {isOpen && (
        <div className="mt-2 space-y-2 text-[13px] text-gray-800">
          {/* The turn step by step, in the order the backend built
              (`services::chat::trace_steps`, shared with the CLI and the eval
              report). Each step is one line; its prompt, output and numbers
              (latency, prefill, KV cache) sit in sections collapsed under it. */}
          <div>
            <div className="text-gray-600 uppercase tracking-wide text-xs mb-0.5">
              {t('chat:reasoning.trace.workflow')} · {formatLatency(trace.totalElapsedMs)}
            </div>
            <ResearchHeader trace={trace} />
            <ol className="space-y-0.5">
              {flow.map((step, i) => (
                <FlowStepRow key={step.key} step={step} index={i} trace={trace} />
              ))}
            </ol>
          </div>
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
