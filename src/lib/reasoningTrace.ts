import type { ChatTrace, LlmCallTrace, ToolCallTrace } from '@/types';

/** Format a millisecond duration as "1.2s" (>= 1s) or "850ms". */
export function formatLatency(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

/** Turn-level throughput. Returns 0 when timing or token data is missing so
 *  callers can branch on a falsy value instead of guarding NaN/Infinity. */
export function tokensPerSecond(tokens: number | null | undefined, ms: number | null | undefined): number {
  if (!tokens || !ms || ms <= 0) {
    return 0;
  }
  return tokens / (ms / 1000);
}

export type PhaseGroup = 'retrieval' | 'llm' | 'tools';

/** i18n key suffix under `chat:reasoning.trace.phase.*`. Kept as a literal
 *  union so callers can build the typed translation key without casting. */
export type PhaseLabelKey = 'retrieval' | 'toolRound' | 'finalStream' | 'toolLoop' | 'llmStreaming' | 'llmCall';

/** One bar in the latency waterfall. `fraction` is the share of the turn's
 *  wall-clock (0..1) used to size the bar. A phase carries either a `labelKey`
 *  (i18n key suffix under `chat:reasoning.trace.phase.*`, for retrieval/LLM
 *  phases) or a literal `label` (a tool-call name, which isn't translatable). */
export interface WaterfallPhase {
  id: string;
  labelKey?: PhaseLabelKey;
  label?: string;
  labelParams?: Record<string, number>;
  ms: number;
  group: PhaseGroup;
  fraction: number;
  failed?: boolean;
}

/** Decompose a turn into the ordered phases that consumed wall-clock time, so
 *  the panel can render a proportional waterfall instead of a flat ms list.
 *
 *  Prefers the per-LLM-call breakdown (each tool round + the final stream) when
 *  present — that's the granular view a developer wants. Falls back to the
 *  aggregate tool-loop / streaming timings for older traces without `llmCalls`.
 *
 *  Bars are sized against `max(totalElapsedMs, sum(phase ms))` so they never
 *  overflow the track even when per-phase timings slightly exceed the total. */
export function buildWaterfall(trace: ChatTrace): WaterfallPhase[] {
  const phases: Omit<WaterfallPhase, 'fraction'>[] = [];

  const r = trace.retrieval;
  if (r && r.elapsedMs > 0) {
    phases.push({ id: 'retrieval', labelKey: 'retrieval', ms: r.elapsedMs, group: 'retrieval' });
  }

  // Tool-call execution is its own wall-clock segment (the round latency does
  // not include it), so each tool gets its own bar interleaved right after the
  // round that requested it.
  let toolIdx = 0;
  const toolPhase = (tc: ToolCallTrace): Omit<WaterfallPhase, 'fraction'> => ({
    id: `tool-${toolIdx++}`,
    label: tc.name,
    ms: tc.elapsedMs,
    group: 'tools',
  });

  const calls = trace.llmCalls ?? [];
  if (calls.length > 0) {
    const { items, unattributed } = groupWorkflow(calls, trace.toolCalls);
    for (const { call: c, tools } of items) {
      const isToolRound = c.kind === 'tool_round';
      phases.push({
        id: `${c.kind}-${c.round}`,
        labelKey: isToolRound ? 'toolRound' : c.kind === 'final_stream' ? 'finalStream' : 'llmCall',
        labelParams: isToolRound ? { n: c.round } : undefined,
        ms: c.latencyMs,
        group: 'llm',
        failed: c.failed,
      });
      for (const tc of tools) {
        phases.push(toolPhase(tc));
      }
    }
    for (const tc of unattributed) {
      phases.push(toolPhase(tc));
    }
  } else if (trace.toolLoopMs != null && trace.toolLoopMs > 0) {
    phases.push({ id: 'tool-loop', labelKey: 'toolLoop', ms: trace.toolLoopMs, group: 'tools' });
  } else if (trace.llmStreamingMs != null && trace.llmStreamingMs > 0) {
    phases.push({ id: 'llm-streaming', labelKey: 'llmStreaming', ms: trace.llmStreamingMs, group: 'llm' });
  }

  const sum = phases.reduce((acc, p) => acc + p.ms, 0);
  const denom = Math.max(trace.totalElapsedMs, sum, 1);
  return phases.map((p) => ({ ...p, fraction: p.ms / denom }));
}

/** One LLM call paired with the tool calls it triggered. */
export interface WorkflowItem {
  call: LlmCallTrace;
  tools: ToolCallTrace[];
}

/** Interleave the per-round LLM calls with the tool calls each round
 *  requested, so the panel can render the flow top-to-bottom instead of
 *  splitting LLM rounds and tool calls into two disconnected lists.
 *
 *  `trace.toolCalls` is a flat list in execution order with no back-pointer to
 *  the round that issued it, but each `tool_round` carries `toolCallsRequested`
 *  — so we walk the rounds in order and hand each one that many tool calls off
 *  the front of the queue. Any tool calls left over (counts didn't add up, or
 *  there were no LLM-call records at all) are returned as `unattributed` so the
 *  caller can still surface them rather than silently dropping them. */
export function groupWorkflow(
  llmCalls: LlmCallTrace[],
  toolCalls: ToolCallTrace[],
): { items: WorkflowItem[]; unattributed: ToolCallTrace[] } {
  let cursor = 0;
  const items = llmCalls.map((call) => {
    const requested = call.kind === 'tool_round' ? (call.toolCallsRequested ?? 0) : 0;
    const tools = toolCalls.slice(cursor, cursor + requested);
    cursor += tools.length;
    return { call, tools };
  });
  return { items, unattributed: toolCalls.slice(cursor) };
}
