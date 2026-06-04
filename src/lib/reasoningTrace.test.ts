import { describe, expect, it } from 'vitest';
import type { ChatTrace, LlmCallTrace, RetrievalTrace, ToolCallTrace } from '@/types';
import { buildWaterfall, formatLatency, groupWorkflow, tokensPerSecond } from './reasoningTrace';

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

function toolCall(name: string): ToolCallTrace {
  return { name, arguments: {}, resultPreview: '', resultChars: 0, elapsedMs: 0 };
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

describe('buildWaterfall', () => {
  it('renders a single full-width bar for a thread-bound turn with one slow LLM round', () => {
    const trace = makeTrace({
      totalElapsedMs: 30000,
      toolLoopMs: 30000,
      llmCalls: [llmCall({ kind: 'tool_round', round: 0, latencyMs: 30000 })],
    });

    const phases = buildWaterfall(trace);

    expect(phases).toHaveLength(1);
    expect(phases[0]).toMatchObject({ group: 'llm', labelKey: 'toolRound', labelParams: { n: 0 }, ms: 30000 });
    expect(phases[0].fraction).toBe(1);
  });

  it('sizes retrieval and final-stream bars proportionally to the total', () => {
    const retrieval: RetrievalTrace = {
      vectorHits: 5,
      ftsHits: 3,
      fusedTopK: 8,
      elapsedMs: 200,
      vectorFallback: false,
    };
    const trace = makeTrace({
      route: { mode: 'rag_first', reason: '', matchedKeywords: [], classifier: 'heuristic' },
      retrieval,
      totalElapsedMs: 1000,
      llmStreamingMs: 800,
      llmCalls: [llmCall({ kind: 'final_stream', round: -1, latencyMs: 800 })],
    });

    const phases = buildWaterfall(trace);

    expect(phases.map((p) => p.group)).toEqual(['retrieval', 'llm']);
    expect(phases[0].fraction).toBeCloseTo(0.2);
    expect(phases[1].fraction).toBeCloseTo(0.8);
    expect(phases[1].labelKey).toBe('finalStream');
  });

  it('never lets a bar overflow the track when phase timings exceed the total', () => {
    const trace = makeTrace({
      totalElapsedMs: 1000,
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, latencyMs: 700 }),
        llmCall({ kind: 'final_stream', round: -1, latencyMs: 500 }),
      ],
    });

    const phases = buildWaterfall(trace);
    const sum = phases.reduce((acc, p) => acc + p.fraction, 0);

    expect(phases.every((p) => p.fraction <= 1)).toBe(true);
    expect(sum).toBeCloseTo(1);
  });

  it('falls back to the aggregate tool-loop timing when no per-call breakdown exists', () => {
    const trace = makeTrace({ totalElapsedMs: 5000, toolLoopMs: 5000 });

    const phases = buildWaterfall(trace);

    expect(phases).toHaveLength(1);
    expect(phases[0]).toMatchObject({ group: 'tools', labelKey: 'toolLoop', ms: 5000 });
  });

  it('marks a failed LLM round so the bar can be styled as an error', () => {
    const trace = makeTrace({
      totalElapsedMs: 100,
      llmCalls: [llmCall({ kind: 'tool_round', round: 0, latencyMs: 100, failed: true })],
    });

    expect(buildWaterfall(trace)[0].failed).toBe(true);
  });

  it('interleaves tool-call bars right after the round that requested them', () => {
    const trace = makeTrace({
      totalElapsedMs: 77000,
      toolCalls: [{ name: 'generate_email_draft', arguments: {}, resultPreview: '', resultChars: 0, elapsedMs: 19000 }],
      llmCalls: [
        llmCall({ kind: 'tool_round', round: 0, latencyMs: 34600, toolCallsRequested: 1 }),
        llmCall({ kind: 'tool_round', round: 1, latencyMs: 23500 }),
      ],
    });

    const phases = buildWaterfall(trace);

    expect(phases.map((p) => p.group)).toEqual(['llm', 'tools', 'llm']);
    expect(phases[1]).toMatchObject({ group: 'tools', label: 'generate_email_draft', ms: 19000 });
    // round0 + tool + round1 = 77.1s ≈ total, so each bar is sized against the real total.
    expect(phases[0].fraction).toBeCloseTo(34600 / 77100);
    expect(phases[1].fraction).toBeCloseTo(19000 / 77100);
  });

  it('still surfaces tool-call bars that no round claimed', () => {
    const trace = makeTrace({
      totalElapsedMs: 1000,
      toolCalls: [{ name: 'orphan', arguments: {}, resultPreview: '', resultChars: 0, elapsedMs: 200 }],
      llmCalls: [llmCall({ kind: 'final_stream', round: -1, latencyMs: 800 })],
    });

    const phases = buildWaterfall(trace);

    expect(phases.map((p) => p.label ?? p.labelKey)).toEqual(['finalStream', 'orphan']);
  });
});

describe('groupWorkflow', () => {
  it('attributes each tool call to the round that requested it, in execution order', () => {
    const llmCalls = [
      llmCall({ kind: 'tool_round', round: 0, toolCallsRequested: 1 }),
      llmCall({ kind: 'tool_round', round: 1, toolCallsRequested: 2 }),
      llmCall({ kind: 'final_stream', round: -1 }),
    ];
    const toolCalls = [toolCall('search'), toolCall('fetch'), toolCall('draft')];

    const { items, unattributed } = groupWorkflow(llmCalls, toolCalls);

    expect(items.map((i) => i.tools.map((t) => t.name))).toEqual([['search'], ['fetch', 'draft'], []]);
    expect(unattributed).toEqual([]);
  });

  it('gives a final-stream call no tool calls even if it carries a stray count', () => {
    const llmCalls = [llmCall({ kind: 'final_stream', round: -1, toolCallsRequested: 3 })];
    const toolCalls = [toolCall('search')];

    const { items, unattributed } = groupWorkflow(llmCalls, toolCalls);

    expect(items[0].tools).toEqual([]);
    expect(unattributed).toEqual([toolCall('search')]);
  });

  it('returns tool calls that no round claimed as unattributed so nothing is hidden', () => {
    const llmCalls = [llmCall({ kind: 'tool_round', round: 0, toolCallsRequested: 1 })];
    const toolCalls = [toolCall('search'), toolCall('orphan')];

    const { items, unattributed } = groupWorkflow(llmCalls, toolCalls);

    expect(items[0].tools.map((t) => t.name)).toEqual(['search']);
    expect(unattributed.map((t) => t.name)).toEqual(['orphan']);
  });

  it('treats a missing request count as zero rather than swallowing tool calls', () => {
    const llmCalls = [llmCall({ kind: 'tool_round', round: 0 })];
    const toolCalls = [toolCall('search')];

    const { items, unattributed } = groupWorkflow(llmCalls, toolCalls);

    expect(items[0].tools).toEqual([]);
    expect(unattributed.map((t) => t.name)).toEqual(['search']);
  });

  it('reports all tool calls as unattributed when there are no LLM-call records', () => {
    const { items, unattributed } = groupWorkflow([], [toolCall('search')]);

    expect(items).toEqual([]);
    expect(unattributed.map((t) => t.name)).toEqual(['search']);
  });
});
