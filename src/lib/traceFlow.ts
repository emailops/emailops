// The reasoning panel's flow: the backend's `trace.steps` (execution order,
// shared with the CLI and the eval report) turned into numbered, one-line
// steps. Each step keeps its prompt, output and numbers for the sections the
// panel collapses under it, so the title line stays short.

import { formatLatency } from '@/lib/reasoningTrace';
import type { ChatTrace, LlmCallTrace, TraceStep } from '@/types';

/** Research-mode phase a step belongs to; `null` on an ordinary turn. */
export type FlowPhase = 'gather' | 'map' | 'condense' | 'reduce';

export type FlowKind = 'router' | 'planner' | 'retrieval' | 'help' | 'tool' | 'llmRound' | 'answer' | 'llmCall';

/** Label ids of the numbers in a step's details section (translated by the panel). */
export type FlowDetailLabel =
  | 'latency'
  | 'prefill'
  | 'kvCache'
  | 'cache'
  | 'toolCalls'
  | 'chars'
  | 'hits'
  | 'route'
  | 'reason'
  | 'status';

export interface FlowDetail {
  label: FlowDetailLabel;
  value: string;
}

export interface FlowStep {
  key: string;
  kind: FlowKind;
  phase: FlowPhase | null;
  /** Position within its phase, for "LLM call 3/12". */
  ordinal: { n: number; total: number } | null;
  /** Tool-loop round, for "LLM round 2". */
  round: number | null;
  /** Short, untranslated result: a call signature, keywords, a count. */
  summary: string | null;
  /** Findings a map batch kept. */
  findings: number | null;
  prompt: string | null;
  output: string | null;
  details: FlowDetail[];
  failed: boolean;
  /** The backend step it came from, for panels that render it themselves. */
  step: TraceStep;
}

const MAX_CALL_CHARS = 120;

/** `name(k: v, …)` from a JSON argument object, empty values skipped. */
export function formatCall(name: string, args: unknown): string {
  const parts =
    args && typeof args === 'object'
      ? Object.entries(args as Record<string, unknown>)
          .filter(([, v]) => v !== null && v !== undefined && v !== '')
          .map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v)}`)
      : [];
  const inner = parts.join(', ');
  const cut = inner.length > MAX_CALL_CHARS ? `${inner.slice(0, MAX_CALL_CHARS - 1)}…` : inner;
  return `${name}(${cut})`;
}

const PHASE_BY_KIND: Record<string, FlowPhase> = {
  research_map: 'map',
  research_condense: 'condense',
  research_reduce: 'reduce',
};

/** The planner's output as the search it planned, or as it came. */
function plannerSummary(output: string | null | undefined): string | null {
  if (!output) return null;
  try {
    const plan = JSON.parse(output) as unknown;
    if (plan && typeof plan === 'object' && !Array.isArray(plan)) return formatCall('search_emails', plan);
  } catch {
    // Not JSON: an outcome line ("planner: search", "no filter …").
  }
  return output;
}

function llmDetails(call: LlmCallTrace, step: Extract<TraceStep, { type: 'llm' }>): FlowDetail[] {
  const details: FlowDetail[] = [{ label: 'latency', value: formatLatency(call.latencyMs) }];
  if (call.prefillMs != null) details.push({ label: 'prefill', value: formatLatency(call.prefillMs) });
  if (step.kvCache) {
    details.push({
      label: 'kvCache',
      value: `${step.kvCache.cached}/${step.kvCache.total} tok (${step.kvCache.pct}%)`,
    });
  }
  if (step.cacheAction) details.push({ label: 'cache', value: step.cacheAction.detail });
  if (call.kind === 'tool_round' && (call.toolCallsRequested ?? 0) > 0) {
    details.push({ label: 'toolCalls', value: String(call.toolCallsRequested) });
  }
  return details;
}

/** Build the panel's flow from a trace. Pure. */
export function buildFlow(trace: ChatTrace): FlowStep[] {
  const steps = trace.steps ?? [];
  const research = trace.research != null;
  // Totals per phase, so each call can say "n of total".
  const totals: Partial<Record<FlowPhase, number>> = {};
  for (const call of trace.llmCalls ?? []) {
    const phase = PHASE_BY_KIND[call.kind];
    if (phase) totals[phase] = (totals[phase] ?? 0) + 1;
  }
  const seen: Partial<Record<FlowPhase, number>> = {};
  const flow: FlowStep[] = [];
  const base = (step: TraceStep, key: string) => ({
    key,
    phase: null as FlowPhase | null,
    ordinal: null,
    round: null,
    summary: null as string | null,
    findings: null,
    prompt: null,
    output: null,
    details: [] as FlowDetail[],
    failed: false,
    step,
  });

  steps.forEach((step, i) => {
    const key = `${step.type}-${i}`;
    switch (step.type) {
      case 'research':
        // Rendered as the flow's header, not as a numbered step.
        return;
      case 'route': {
        const r = trace.route;
        const details: FlowDetail[] = [];
        if (!research) details.push({ label: 'route', value: r.mode });
        if (r.reason) details.push({ label: 'reason', value: r.reason });
        flow.push({
          ...base(step, key),
          kind: 'router',
          summary: r.matchedKeywords.length > 0 ? r.matchedKeywords.join(', ') : null,
          details,
        });
        return;
      }
      case 'retrieval': {
        const r = trace.retrieval;
        flow.push({
          ...base(step, key),
          kind: 'retrieval',
          summary: r ? `${r.vectorHits} vec + ${r.ftsHits} fts → ${r.fusedTopK}` : null,
          details: r ? [{ label: 'latency', value: formatLatency(r.elapsedMs) }] : [],
        });
        return;
      }
      case 'help': {
        const h = trace.help;
        flow.push({
          ...base(step, key),
          kind: 'help',
          summary: h ? `${h.included}/${h.candidates}` : null,
          details: h ? [{ label: 'latency', value: formatLatency(h.elapsedMs) }] : [],
        });
        return;
      }
      case 'tool': {
        const call = trace.toolCalls[step.index];
        if (!call) return;
        const gather = research && call.round <= -3;
        const signature = formatCall(call.name, call.arguments);
        flow.push({
          ...base(step, key),
          kind: 'tool',
          phase: gather ? 'gather' : null,
          summary: gather && call.resultPreview ? `${signature} → ${call.resultPreview}` : signature,
          output: gather ? null : call.resultPreview || null,
          details: [
            { label: 'latency', value: formatLatency(call.elapsedMs) },
            { label: 'chars', value: String(call.resultChars) },
          ].filter((d) => !gather || d.label === 'latency') as FlowDetail[],
        });
        return;
      }
      case 'llm': {
        const call = trace.llmCalls?.[step.index];
        if (!call) return;
        const phase = PHASE_BY_KIND[call.kind] ?? null;
        const common = {
          ...base(step, key),
          prompt: call.input ?? null,
          output: call.output ?? null,
          details: llmDetails(call, step),
          failed: call.failed ?? false,
        };
        if (call.kind === 'planner') {
          flow.push({
            ...common,
            kind: 'planner',
            phase: research ? 'gather' : null,
            summary: plannerSummary(call.output),
          });
        } else if (phase) {
          seen[phase] = (seen[phase] ?? 0) + 1;
          const findings =
            phase === 'map' ? (call.output ?? '').split('\n').filter((l) => l.includes('email://')).length : null;
          flow.push({
            ...common,
            kind: 'llmCall',
            phase,
            ordinal: { n: seen[phase] ?? 1, total: totals[phase] ?? 1 },
            findings,
          });
        } else if (call.kind === 'tool_round') {
          flow.push({ ...common, kind: 'llmRound', round: call.round });
        } else {
          flow.push({ ...common, kind: call.kind === 'final_stream' ? 'answer' : 'llmCall' });
        }
        return;
      }
    }
  });
  return flow;
}
