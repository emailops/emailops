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

/** Human-readable summary of what this call did to the cache. Returns null
 *  when the provider didn't report a `prefixPlan` (HTTP providers / legacy
 *  traces) so the UI hides the segment. */
export interface CacheAction {
  /** One of `"extend" | "anchor-hit" | "wiped" | "cold-fresh"`. The UI maps
   *  these to badge labels + colors. */
  kind: 'extend' | 'anchor-hit' | 'wiped' | 'cold-fresh';
  /** One-line explanation suitable for inline display next to the call. */
  detail: string;
}

/** Describe what happened to the seq-2 anchor across one call. Pure helper —
 *  unit-tested with the rest of `cacheAction`'s cases. */
function describeAnchor(sysBefore: number, sysAfter: number): string {
  if (sysBefore === 0 && sysAfter === 0) {
    // Anchor stayed empty. Most likely means runtime returned sys_tok=0
    // (no leading system message, or `system_prefix_bytes` returned None).
    return `no anchor seeded (sys_tok=0 — system prefix not detected this call)`;
  }
  if (sysBefore === 0 && sysAfter > 0) return `anchor seeded for the first time (${sysAfter} tok)`;
  if (sysBefore > 0 && sysAfter === 0) {
    // The anchor was cleared (ColdPrefill or Extend reseed branch) but
    // plan_anchor_seed didn't put anything back. Diagnostic worth flagging.
    return `anchor wiped (was ${sysBefore} tok) and NOT reseeded — runtime returned sys_tok=0`;
  }
  if (sysBefore === sysAfter) return `anchor unchanged (${sysAfter} tok)`;
  // Both non-zero but different — wipe + reseed with a different system block,
  // or the anchor grew/shrunk within an in-conversation Extend.
  const arrow = sysBefore < sysAfter ? '↑' : '↓';
  return `anchor resized ${sysBefore}→${sysAfter} tok ${arrow}`;
}

export function cacheAction(call: LlmCallTrace): CacheAction | null {
  if (!call.prefixPlan) return null;
  const sysBefore = call.sysCachedBefore ?? 0;
  const sysAfter = call.sysCachedAfter ?? 0;
  const dropped = call.droppedFrontTokens ?? 0;
  const anchor = describeAnchor(sysBefore, sysAfter);

  // Front-truncation is the dominant cause of cold prefills on long chats —
  // surface it loudly so the user doesn't blame the cache. Each truncated
  // round rewrites the leading bytes of the prompt, which makes cache reuse
  // structurally impossible regardless of the plan logic. Show this message
  // for ANY plan: cold-prefill, restart-from-anchor (rare), even extend
  // (rarer) — the user wants to know that out-of-context happened.
  if (dropped > 0) {
    return {
      kind: 'wiped',
      detail: `out of context: front-truncated by ${dropped} tokens — leading bytes rewrote, cache cannot follow (anchor not seeded)`,
    };
  }

  if (call.prefixPlan === 'Extend') {
    // Extend reuses seq 0; but the anchor can still be seeded mid-call when
    // sysBefore=0 (first-ever cached call, or right after an aux call evicted
    // it). Use cold-fresh as the icon in that case so the user sees the
    // "first time" event.
    const kind: CacheAction['kind'] = sysBefore === 0 && sysAfter > 0 ? 'cold-fresh' : 'extend';
    return { kind, detail: `extended seq 0 · ${anchor}` };
  }
  if (call.prefixPlan === 'RestartFromAnchor') {
    return { kind: 'anchor-hit', detail: `anchor hit: reused ${sysAfter} tokens (cross-conversation) · ${anchor}` };
  }
  // ColdPrefill — distinguish "anchor wiped" (regression) from a fresh start.
  const kind: CacheAction['kind'] = sysBefore > 0 ? 'wiped' : 'cold-fresh';
  return { kind, detail: `cold prefill · ${anchor}` };
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
 *  - For each `tool_round` LLM call at round R, the matching tools are those
 *    with `tool.round === R`. The backend pushes tools onto `trace.toolCalls`
 *    in execution order, so we walk the looped tools with a forward cursor and
 *    drain ALL tools whose `round` equals R (covering the XML/python-call
 *    salvage case where `toolCallsRequested` is 0 but the loop still dispatched
 *    tools after the salvage). Once a tool is consumed it cannot match a later
 *    round.
 *  - Legacy traces (persisted before the `round` field existed) deserialize
 *    every tool to round 0. For those we fall back to the `toolCallsRequested`
 *    cursor so each round still gets the right slice. A trace is treated as
 *    "legacy" iff every looped tool has `round === 0` AND the sum of LLM
 *    `toolCallsRequested` values is ≥ the looped-tool count (the legacy invariant).
 *  - `final_stream` calls never own tool calls.
 *  - Any tool left over (no matching round / cursor exhausted with leftovers)
 *    is surfaced at the end rather than hidden. */
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

  // Decide whether to drive ordering off `tool.round` (modern traces) or the
  // legacy `toolCallsRequested` cursor (every looped tool defaults to round 0).
  const allLoopedAtZero = looped.length > 0 && looped.every((tc) => tc.round === 0);
  const requestedSum = llmCalls.reduce((acc, c) => acc + (c.toolCallsRequested ?? 0), 0);
  const useLegacyCursor = allLoopedAtZero && requestedSum >= looped.length;

  let cursor = 0;
  const consumed = new Set<number>();

  for (const call of llmCalls) {
    entries.push({ kind: 'llm', id: `${call.kind}-${call.round}`, call });
    if (call.kind !== 'tool_round') continue;

    if (useLegacyCursor) {
      // Legacy path: take this round's slice via toolCallsRequested.
      const requested = call.toolCallsRequested ?? 0;
      for (let i = 0; i < requested && cursor < looped.length; i += 1) {
        emitTool(looped[cursor]);
        consumed.add(cursor);
        cursor += 1;
      }
    } else {
      // Modern path: emit every tool whose round matches this LLM call's round.
      // Walks forward only so an early round can't claim a later tool.
      for (let i = cursor; i < looped.length; i += 1) {
        if (looped[i].round === call.round) {
          emitTool(looped[i]);
          consumed.add(i);
          cursor = i + 1;
        } else if (looped[i].round > call.round) {
          // Later round — stop scanning, leave for a future LLM round.
          break;
        }
      }
    }
  }

  // Leftover tools that no round claimed (e.g. round number outside the LLM
  // range, salvage that lagged the LLM trace).
  for (let i = 0; i < looped.length; i += 1) {
    if (!consumed.has(i)) emitTool(looped[i]);
  }

  return entries;
}
