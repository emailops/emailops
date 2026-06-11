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

/** KV-cache reuse for one LLM call, ready for display. */
export interface KvCacheStats {
  /** Prompt tokens served from the reused KV-cache prefix. */
  cached: number;
  /** Total prompt tokens for the call. */
  total: number;
  /** Whole-number percentage of the prompt served from cache. */
  pct: number;
}

/** Cache reuse stats for an LLM call, or null when the provider doesn't
 *  report them (HTTP providers) — callers hide the segment entirely then.
 *  A reported 0/N is meaningful (cold prefill) and IS returned. */
export function kvCacheStats(call: LlmCallTrace): KvCacheStats | null {
  if (call.cachedPromptTokens == null || !call.promptTokens) {
    return null;
  }
  return {
    cached: call.cachedPromptTokens,
    total: call.promptTokens,
    pct: Math.round((call.cachedPromptTokens / call.promptTokens) * 100),
  };
}

/** One step in the turn's execution flow — either an LLM round-trip or a tool
 *  call — tagged so the panel can render a single ordered list. */
export type FlowEntry =
  | { kind: 'llm'; id: string; call: LlmCallTrace }
  | { kind: 'tool'; id: string; call: ToolCallTrace };

/** Flatten a turn into the LLM calls and tool calls it ran, in true execution
 *  order, so the reasoning panel reads top-to-bottom the way the turn actually
 *  happened.
 *
 *  Ordering rules:
 *  - Preseeded shortcut tools (round < 0) run before the LLM loop, so they lead.
 *    The `round` field exists only to flag these — they're the case the model
 *    never asked for, so request counts can't place them.
 *  - The remaining tools are handed out across the `tool_round` LLM calls in
 *    order, each round taking as many as its `toolCallsRequested` count. This
 *    is the signal that's reliable for every trace, including legacy ones
 *    persisted before `round` existed (where every tool defaults to round 0).
 *  - `final_stream` calls never own tool calls.
 *  - Any tool left over (request counts didn't add up, or there were no
 *    `tool_round` records) is surfaced at the end rather than hidden.
 *
 *  `trace.toolCalls` is in execution order, so a single forward cursor over the
 *  looped (non-preseeded) tools suffices. */
export function buildFlow(trace: ChatTrace): FlowEntry[] {
  const toolCalls = trace.toolCalls ?? [];
  const llmCalls = trace.llmCalls ?? [];
  const preseeded = toolCalls.filter((tc) => tc.round < 0);
  const looped = toolCalls.filter((tc) => tc.round >= 0);

  const entries: FlowEntry[] = [];
  let id = 0;
  const emitTool = (call: ToolCallTrace) => {
    entries.push({ kind: 'tool', id: `tool-${id}`, call });
    id += 1;
  };

  // Preseeded shortcut tools ran before any LLM call.
  for (const call of preseeded) {
    emitTool(call);
  }

  let cursor = 0;
  for (const call of llmCalls) {
    entries.push({ kind: 'llm', id: `${call.kind}-${call.round}`, call });
    if (call.kind === 'tool_round') {
      const requested = call.toolCallsRequested ?? 0;
      for (let i = 0; i < requested && cursor < looped.length; i += 1) {
        emitTool(looped[cursor]);
        cursor += 1;
      }
    }
  }

  // Leftover tools that no round claimed — surface them rather than hide them.
  while (cursor < looped.length) {
    emitTool(looped[cursor]);
    cursor += 1;
  }

  return entries;
}
