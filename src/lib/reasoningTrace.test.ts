import { describe, expect, it } from 'vitest';
import type { ChatTrace, LlmCallTrace, ToolCallTrace } from '@/types';
import { buildFlow, formatLatency, kvCacheStats, tokensPerSecond } from './reasoningTrace';

function makeTrace(overrides: Partial<ChatTrace> = {}): ChatTrace {
  return {
    route: { mode: 'tools_first', reason: 'thread-bound', matchedKeywords: [], classifier: 'forced' },
    retrieval: null,
    toolCalls: [],
    model: 'qwen3.5-4b',
    totalElapsedMs: 0,
    ...overrides,
  };
}

function llmCall(overrides: Partial<LlmCallTrace>): LlmCallTrace {
  return { kind: 'tool_round', round: 0, latencyMs: 0, ...overrides };
}

function toolCall(name: string, round = 0): ToolCallTrace {
  return { name, round, arguments: {}, resultPreview: '', resultChars: 0, elapsedMs: 0 };
}

/** Collapse a flow into a readable `kind:label` list for order assertions. */
function flowLabels(trace: ChatTrace): string[] {
  return buildFlow(trace).map((e) => (e.kind === 'llm' ? `llm:${e.call.kind}:${e.call.round}` : `tool:${e.call.name}`));
}

describe('formatLatency', () => {
  it('renders sub-second durations in ms', () => {
    expect(formatLatency(500)).toBe('500ms');
  });

  it('renders durations >= 1s with one decimal', () => {
    expect(formatLatency(30000)).toBe('30.0s');
  });
});

describe('tokensPerSecond', () => {
  it('divides tokens by seconds', () => {
    expect(tokensPerSecond(300, 30000)).toBe(10);
  });

  it('returns 0 when the duration is zero to avoid Infinity', () => {
    expect(tokensPerSecond(50, 0)).toBe(0);
  });

  it('returns 0 when token count is missing', () => {
    expect(tokensPerSecond(null, 1000)).toBe(0);
  });
});

describe('buildFlow', () => {
  it('places a preseeded shortcut tool before the LLM round that consumed it', () => {
    // Shortcut path: search_emails runs first (round -1), then tool_round 0
    // synthesises the answer and requests no further tools. Execution order is
    // tool → LLM, so the flow must read that way (the bug was the reverse).
    const trace = makeTrace({
      totalElapsedMs: 35000,
      toolCalls: [toolCall('search_emails', -1)],
      llmCalls: [llmCall({ kind: 'tool_round', round: 0, latencyMs: 35000, toolCallsRequested: 0 })],
    });

    expect(flowLabels(trace)).toEqual(['tool:search_emails', 'llm:tool_round:0']);
  });

  it('places a normal-loop tool right after the round that requested it', () => {
    const trace = makeTrace({
      totalElapsedMs: 77000,
      toolCalls: [toolCall('generate_email_draft', 0)],
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, latencyMs: 34600, toolCallsRequested: 1 }),
        llmCall({ kind: 'tool_round', round: 1, latencyMs: 23500 }),
      ],
    });

    expect(flowLabels(trace)).toEqual(['llm:tool_round:0', 'tool:generate_email_draft', 'llm:tool_round:1']);
  });

  it('orders multi-round tool calls by the round that issued each one', () => {
    const trace = makeTrace({
      toolCalls: [toolCall('search', 0), toolCall('fetch', 1), toolCall('draft', 1)],
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, toolCallsRequested: 1 }),
        llmCall({ kind: 'tool_round', round: 1, toolCallsRequested: 2 }),
        llmCall({ kind: 'final_stream', round: -1 }),
      ],
    });

    expect(flowLabels(trace)).toEqual([
      'llm:tool_round:0',
      'tool:search',
      'llm:tool_round:1',
      'tool:fetch',
      'tool:draft',
      'llm:final_stream:-1',
    ]);
  });

  it('interleaves per-round tools by request count even when legacy tools all default to round 0', () => {
    // Traces persisted before the `round` field existed deserialize every tool
    // to round 0. The flow must still place each round's tool after that round
    // using the reliable per-round request count, not the tool's round number
    // (otherwise all tools cluster under round 0 — the legacy-trace bug).
    const trace = makeTrace({
      toolCalls: [toolCall('search_contacts', 0), toolCall('search_emails', 0), toolCall('search_emails', 0)],
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, toolCallsRequested: 1 }),
        llmCall({ kind: 'tool_round', round: 1, toolCallsRequested: 1 }),
        llmCall({ kind: 'tool_round', round: 2, toolCallsRequested: 1 }),
        llmCall({ kind: 'final_stream', round: -1 }),
      ],
    });

    expect(flowLabels(trace)).toEqual([
      'llm:tool_round:0',
      'tool:search_contacts',
      'llm:tool_round:1',
      'tool:search_emails',
      'llm:tool_round:2',
      'tool:search_emails',
      'llm:final_stream:-1',
    ]);
  });

  it('interleaves a preseeded tool and a tool the first round then requests', () => {
    const trace = makeTrace({
      toolCalls: [toolCall('search_emails', -1), toolCall('get_thread', 0)],
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, toolCallsRequested: 1 }),
        llmCall({ kind: 'final_stream', round: -1 }),
      ],
    });

    expect(flowLabels(trace)).toEqual([
      'tool:search_emails',
      'llm:tool_round:0',
      'tool:get_thread',
      'llm:final_stream:-1',
    ]);
  });

  it('never attaches tool calls to a final-stream LLM call', () => {
    const trace = makeTrace({
      toolCalls: [toolCall('search', 0)],
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, toolCallsRequested: 1 }),
        llmCall({ kind: 'final_stream', round: -1 }),
      ],
    });

    expect(flowLabels(trace)).toEqual(['llm:tool_round:0', 'tool:search', 'llm:final_stream:-1']);
  });

  it('surfaces tool calls that no round claimed at the end instead of hiding them', () => {
    const trace = makeTrace({
      toolCalls: [toolCall('orphan', 5)],
      llmCalls: [llmCall({ kind: 'final_stream', round: -1 })],
    });

    expect(flowLabels(trace)).toEqual(['llm:final_stream:-1', 'tool:orphan']);
  });

  it('returns an empty flow when there are no LLM or tool calls', () => {
    expect(buildFlow(makeTrace())).toEqual([]);
  });

  it('carries the underlying call objects through so the UI can render details', () => {
    const trace = makeTrace({
      toolCalls: [toolCall('search_emails', -1)],
      llmCalls: [llmCall({ kind: 'tool_round', round: 0, latencyMs: 35000 })],
    });

    const flow = buildFlow(trace);
    expect(flow[0]).toMatchObject({ kind: 'tool', call: { name: 'search_emails' } });
    expect(flow[1]).toMatchObject({ kind: 'llm', call: { kind: 'tool_round', latencyMs: 35000 } });
    // ids are unique so they're safe as React keys.
    expect(new Set(flow.map((e) => e.id)).size).toBe(flow.length);
  });
});

describe('kvCacheStats', () => {
  it('returns cached count and percentage when the call reports cache reuse', () => {
    expect(kvCacheStats(llmCall({ promptTokens: 4382, cachedPromptTokens: 3272 }))).toEqual({
      cached: 3272,
      total: 4382,
      pct: 75,
    });
  });

  it('returns a zero-percent stat for a cold prefill so the UI can show the miss', () => {
    expect(kvCacheStats(llmCall({ promptTokens: 3279, cachedPromptTokens: 0 }))).toEqual({
      cached: 0,
      total: 3279,
      pct: 0,
    });
  });

  it('returns null when the provider reports no cache data (HTTP providers)', () => {
    expect(kvCacheStats(llmCall({ promptTokens: 1200 }))).toBeNull();
    expect(kvCacheStats(llmCall({}))).toBeNull();
  });

  it('returns null when prompt tokens are missing or zero (avoids divide-by-zero)', () => {
    expect(kvCacheStats(llmCall({ cachedPromptTokens: 10 }))).toBeNull();
    expect(kvCacheStats(llmCall({ promptTokens: 0, cachedPromptTokens: 0 }))).toBeNull();
  });
});
