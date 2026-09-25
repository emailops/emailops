// The reasoning panel walks `trace.steps` — the one execution-order list the
// backend builds for the panel, `emailops-cli chat --trace` and the eval
// report (`services::chat::trace_steps`) — instead of ordering the turn itself.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { ChatTrace } from '@/types';
import { ReasoningSection } from './ReasoningTrace';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function trace(): ChatTrace {
  return {
    route: {
      mode: 'tools_first',
      reason: 'planner turned the question into a search filter',
      matchedKeywords: [],
      classifier: 'planner',
    },
    retrieval: { vectorHits: 20, ftsHits: 30, fusedTopK: 9, elapsedMs: 7, vectorFallback: false, ftsSearchMs: 1 },
    toolCalls: [{ name: 'search_emails', round: 0, arguments: {}, resultPreview: '', resultChars: 700, elapsedMs: 4 }],
    model: 'qwen',
    totalElapsedMs: 2100,
    llmCalls: [
      { kind: 'planner', round: -2, latencyMs: 219 },
      { kind: 'tool_round', round: 0, latencyMs: 1800, toolCallsRequested: 1 },
    ],
    // Deliberately not the order `llmCalls`/`toolCalls` would suggest on
    // their own: the panel must follow the steps, not re-derive them.
    steps: [
      { type: 'route' },
      { type: 'llm', index: 0, kvCache: { cached: 10, total: 20, pct: 50 }, cacheAction: null },
      { type: 'retrieval' },
      { type: 'llm', index: 1, kvCache: null, cacheAction: null },
      { type: 'tool', index: 0 },
    ],
  };
}

function openPanel(t: ChatTrace) {
  act(() => root.render(<ReasoningSection trace={t} />));
  act(() => container.querySelector('button')?.click());
}

function rows(): string[] {
  return Array.from(container.querySelectorAll('[data-testid="trace-step"]')).map((el) => el.textContent ?? '');
}

function toggle(row: number, section: 'prompt' | 'output' | 'details') {
  const el = container.querySelectorAll('[data-testid="trace-step"]')[row];
  const btn = el?.querySelector<HTMLButtonElement>(`[data-section="${section}"]`);
  if (!btn) throw new Error(`no ${section} toggle on row ${row}`);
  act(() => btn.click());
}

// i18n is not initialised under vitest, so `t()` returns the key: rows are
// identified by the key their label uses.
describe('ReasoningSection', () => {
  it('renders one numbered row per step, in the order the backend gives', () => {
    openPanel(trace());
    const r = rows();
    expect(r).toHaveLength(5);
    expect(r[0]).toMatch(/^1\./);
    expect(r[0]).toContain('flow.kind.router');
    expect(r[1]).toContain('flow.kind.planner');
    expect(r[2]).toContain('flow.kind.retrieval');
    expect(r[3]).toContain('flow.kind.llmRound');
    expect(r[4]).toContain('search_emails');
    expect(r[4]).toMatch(/^5\./);
  });

  it('names who decided the route instead of printing the raw classifier', () => {
    openPanel(trace());
    const route = rows()[0];
    expect(route).toContain('classifier.planner');
    expect(route).not.toMatch(/·\s*planner/);
  });

  it('keeps latency and KV-cache figures in a details section, collapsed by default', () => {
    openPanel(trace());
    expect(rows()[1]).not.toContain('50%');
    expect(rows()[1]).not.toContain('219ms');
    toggle(1, 'details');
    expect(rows()[1]).toContain('10/20 tok (50%)');
    expect(rows()[1]).toContain('219ms');
  });

  it('tags research steps with their phase under a research header', () => {
    const t = trace();
    t.research = {
      nCtx: 16384,
      plannedEmails: 25,
      searchHits: 25,
      semanticHits: 0,
      emailsAnalyzed: 25,
      batches: 1,
      failedBatches: 0,
      findings: 3,
      relevantEmails: 3,
      condenseCalls: 0,
      stopped: false,
      gatherMs: 1,
      mapMs: 1,
      condenseMs: 0,
      reduceMs: 1,
    };
    t.llmCalls = [
      { kind: 'planner', round: -2, latencyMs: 219, output: '{"subject":"Petición de contacto"}' },
      { kind: 'research_map', round: 0, latencyMs: 1800, input: 'PROMPT', output: '- a (email://e1)' },
      { kind: 'research_reduce', round: -1, latencyMs: 900, output: 'Report' },
    ];
    t.toolCalls = [
      {
        name: 'search_emails',
        round: -3,
        arguments: { subject: 'x' },
        resultPreview: '25 emails',
        resultChars: 9,
        elapsedMs: 4,
      },
    ];
    t.steps = [
      { type: 'research' },
      { type: 'route' },
      { type: 'llm', index: 0, kvCache: null, cacheAction: null },
      { type: 'tool', index: 0 },
      { type: 'llm', index: 1, kvCache: null, cacheAction: null },
      { type: 'llm', index: 2, kvCache: null, cacheAction: null },
    ];
    openPanel(t);
    expect(container.textContent).toContain('flow.researchOn');
    const r = rows();
    expect(r.map((x) => x.match(/\[(\w+)\]/)?.[1] ?? null)).toEqual([null, 'GATHER', 'GATHER', 'MAP', 'REDUCE']);
    expect(r[2]).toContain('search_emails(subject: x) → 25 emails');
    // The prompt is collapsed until asked for.
    expect(r[3]).not.toContain('PROMPT');
    toggle(3, 'prompt');
    expect(rows()[3]).toContain('PROMPT');
  });
});
