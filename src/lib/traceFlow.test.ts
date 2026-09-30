import { describe, expect, it } from 'vitest';
import type { BudgetCut, BudgetTrace, ChatTrace, LlmCallTrace, ToolCallTrace } from '@/types';
import { budgetAffectsAnswer, buildFlow, formatCall } from './traceFlow';

function llm(kind: string, round: number, extra: Partial<LlmCallTrace> = {}): LlmCallTrace {
  return { kind, round, latencyMs: 1000, ...extra };
}

function tool(name: string, round: number, extra: Partial<ToolCallTrace> = {}): ToolCallTrace {
  return { name, round, arguments: {}, resultPreview: '', resultChars: 0, elapsedMs: 5, ...extra };
}

function researchTrace(): ChatTrace {
  return {
    route: {
      mode: 'tools_first',
      reason: 'heuristic matched: todas las',
      matchedKeywords: ['todas las'],
      classifier: 'heuristic',
    },
    toolCalls: [
      tool('search_emails', -3, {
        arguments: { subject: 'Petición de contacto', intent: 'inquiry' },
        resultPreview: '33 emails',
      }),
      tool('search_emails', -3, { arguments: { subject: 'Petición de contacto' }, resultPreview: '40 emails' }),
    ],
    model: 'qwen',
    totalElapsedMs: 105_500,
    llmCalls: [
      llm('planner', -2, { output: '{"subject":"Petición de contacto","intent":"inquiry"}' }),
      llm('research_map', 0, {
        input: 'PROMPT 1',
        output: 'Findings:\n- A (email://e1)\n- B (email://e2)',
        prefillMs: 5400,
        promptTokens: 2739,
        cachedPromptTokens: 0,
      }),
      llm('research_map', 1, { output: 'NONE' }),
      llm('research_condense', 0, { output: '- merged (email://e1)' }),
      llm('research_reduce', -1, { output: 'Report' }),
    ],
    research: {
      nCtx: 16384,
      plannedEmails: 40,
      searchHits: 73,
      semanticHits: 0,
      emailsAnalyzed: 40,
      batches: 2,
      failedBatches: 0,
      findings: 2,
      relevantEmails: 2,
      condenseCalls: 1,
      stopped: false,
      gatherMs: 4300,
      mapMs: 30_000,
      condenseMs: 5000,
      reduceMs: 40_700,
    },
    steps: [
      { type: 'research' },
      { type: 'route' },
      { type: 'llm', index: 0, kvCache: null, cacheAction: null },
      { type: 'tool', index: 0 },
      { type: 'tool', index: 1 },
      {
        type: 'llm',
        index: 1,
        kvCache: { cached: 0, total: 2739, pct: 0 },
        cacheAction: { kind: 'cold-fresh', detail: 'one-shot slot: instructions decoded' },
      },
      { type: 'llm', index: 2, kvCache: null, cacheAction: null },
      { type: 'llm', index: 3, kvCache: null, cacheAction: null },
      { type: 'llm', index: 4, kvCache: null, cacheAction: null },
    ],
  };
}

describe('buildFlow for a research turn', () => {
  it('numbers router, gather, map, condense and reduce steps with their phase', () => {
    const flow = buildFlow(researchTrace());
    expect(flow.map((s) => [s.kind, s.phase])).toEqual([
      ['router', null],
      ['planner', 'gather'],
      ['tool', 'gather'],
      ['tool', 'gather'],
      ['llmCall', 'map'],
      ['llmCall', 'map'],
      ['llmCall', 'condense'],
      ['llmCall', 'reduce'],
    ]);
  });

  it('counts each phase as "n of total"', () => {
    const flow = buildFlow(researchTrace());
    expect(flow[4].ordinal).toEqual({ n: 1, total: 2 });
    expect(flow[5].ordinal).toEqual({ n: 2, total: 2 });
    expect(flow[6].ordinal).toEqual({ n: 1, total: 1 });
    expect(flow[7].ordinal).toEqual({ n: 1, total: 1 });
  });

  it('summarises the planner as the search it planned and a gather as its call and result', () => {
    const flow = buildFlow(researchTrace());
    expect(flow[1].summary).toBe('search_emails(subject: Petición de contacto, intent: inquiry)');
    expect(flow[2].summary).toBe('search_emails(subject: Petición de contacto, intent: inquiry) → 33 emails');
  });

  it('counts the findings a map batch kept', () => {
    const flow = buildFlow(researchTrace());
    expect(flow[4].findings).toBe(2);
    expect(flow[5].findings).toBe(0);
  });

  it('keeps prompt and output for the collapsed sections, and timing and cache only in details', () => {
    const map = buildFlow(researchTrace())[4];
    expect(map.prompt).toBe('PROMPT 1');
    expect(map.output).toContain('(email://e1)');
    expect(map.summary ?? '').not.toMatch(/\d+(\.\d+)?s\b/);
    const labels = map.details.map((d) => d.label);
    expect(labels).toEqual(['latency', 'prefill', 'kvCache', 'cache']);
    expect(map.details.find((d) => d.label === 'cache')?.value).toBe('one-shot slot: instructions decoded');
  });

  it('shows the matched keywords on the router step', () => {
    expect(buildFlow(researchTrace())[0].summary).toBe('todas las');
  });
});

describe('buildFlow for an ordinary turn', () => {
  it('has no phases and names the tool rounds and the answer', () => {
    const trace: ChatTrace = {
      route: { mode: 'tools_first', reason: '', matchedKeywords: [], classifier: 'planner' },
      toolCalls: [tool('search_emails', 0, { arguments: { from: 'a@b.com' }, resultChars: 700 })],
      model: 'qwen',
      totalElapsedMs: 2000,
      llmCalls: [llm('tool_round', 0, { toolCallsRequested: 1 }), llm('final_stream', -1)],
      steps: [
        { type: 'route' },
        { type: 'llm', index: 0, kvCache: null, cacheAction: null },
        { type: 'tool', index: 0 },
        { type: 'llm', index: 1, kvCache: null, cacheAction: null },
      ],
    };
    const flow = buildFlow(trace);
    expect(flow.map((s) => [s.kind, s.phase])).toEqual([
      ['router', null],
      ['llmRound', null],
      ['tool', null],
      ['answer', null],
    ]);
    expect(flow[2].summary).toBe('search_emails(from: a@b.com)');
    expect(flow[2].details.map((d) => d.label)).toEqual(['latency', 'chars']);
  });
});

describe('formatCall', () => {
  it('skips empty arguments and truncates long ones', () => {
    expect(formatCall('f', { a: 'x', b: null, c: '' })).toBe('f(a: x)');
    expect(formatCall('f', {})).toBe('f()');
    expect(formatCall('f', { q: 'y'.repeat(200) }).length).toBeLessThanOrEqual(124);
  });
});

describe('buildFlow — skills', () => {
  it('shows which skill was applied and who chose it', () => {
    const trace: ChatTrace = {
      route: {
        mode: 'rag_first',
        reason: 'planner found no single search',
        matchedKeywords: [],
        classifier: 'planner',
      },
      toolCalls: [],
      model: 'qwen',
      totalElapsedMs: 1000,
      llmCalls: [llm('planner', -2, { output: 'planner: defer' })],
      appliedSkills: [
        { name: 'weekly-report', via: 'planner' },
        { name: 'formal-tone', via: 'slash' },
      ],
      steps: [{ type: 'route' }, { type: 'llm', index: 0, kvCache: null, cacheAction: null }, { type: 'skill' }],
    };
    const flow = buildFlow(trace);
    expect(flow.map((s) => s.kind)).toEqual(['router', 'planner', 'skill']);
    expect(flow[2].summary).toBe('weekly-report (planner), formal-tone (/)');
  });
});

describe('buildFlow — context budget', () => {
  const trace = (budget: ChatTrace['budget']): ChatTrace => ({
    route: { mode: 'rag_first', reason: '', matchedKeywords: [], classifier: 'planner' },
    toolCalls: [],
    model: 'qwen',
    totalElapsedMs: 1000,
    llmCalls: [llm('tool_round', 0)],
    budget,
    steps: [{ type: 'route' }, { type: 'budget' }, { type: 'llm', index: 0, kvCache: null, cacheAction: null }],
  });

  it('shows what was cut as its own step, before the first model call', () => {
    const flow = buildFlow(
      trace({
        nCtx: 8192,
        replyReserve: 1024,
        estimatedPromptTokens: 6000,
        cuts: [{ kind: 'historyTurns', messages: 2 }],
        fits: true,
      }),
    );
    expect(flow.map((s) => s.kind)).toEqual(['router', 'budget', 'llmRound']);
    expect(flow[1].failed).toBe(false);
  });

  it('marks the step failed when the prompt still did not fit', () => {
    const flow = buildFlow(
      trace({ nCtx: 8192, replyReserve: 1024, estimatedPromptTokens: 9000, cuts: [], fits: false }),
    );
    expect(flow[1].failed).toBe(true);
  });
});

describe('budgetAffectsAnswer', () => {
  const budget = (cuts: BudgetCut[], fits = true): BudgetTrace => ({
    nCtx: 8192,
    replyReserve: 1024,
    estimatedPromptTokens: 6000,
    cuts,
    fits,
  });

  it('is false without a budget record, and for history-only cuts', () => {
    expect(budgetAffectsAnswer(null)).toBe(false);
    expect(budgetAffectsAnswer(undefined)).toBe(false);
    expect(
      budgetAffectsAnswer(
        budget([
          { kind: 'historySources', messages: 1 },
          { kind: 'historyTurns', messages: 2 },
        ]),
      ),
    ).toBe(false);
  });

  it('is true when something of this turn was cut, or the prompt did not fit', () => {
    expect(budgetAffectsAnswer(budget([{ kind: 'sourceExcerpts', charsPerEmail: 1200 }]))).toBe(true);
    expect(budgetAffectsAnswer(budget([{ kind: 'sourcesDropped', emails: 2 }]))).toBe(true);
    expect(budgetAffectsAnswer(budget([{ kind: 'openThread', chars: 4000 }]))).toBe(true);
    expect(budgetAffectsAnswer(budget([{ kind: 'toolResults', results: 1, charsDropped: 9000 }]))).toBe(true);
    expect(budgetAffectsAnswer(budget([], false))).toBe(true);
  });
});
