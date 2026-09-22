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

// i18n is not initialised under vitest, so `t()` returns the key: rows are
// identified by the key their label uses.
describe('ReasoningSection', () => {
  it('renders one row per step, in the order the backend gives', () => {
    openPanel(trace());
    const r = rows();
    expect(r).toHaveLength(5);
    expect(r[0]).toContain('routeMode.tools_first');
    expect(r[1]).toContain('phase.planner');
    expect(r[2]).toContain('step.rag');
    expect(r[3]).toContain('phase.toolRound');
    expect(r[4]).toContain('search_emails');
  });

  it('shows the KV-cache figures the step carries', () => {
    // The planner call itself reports no cache numbers; only the step does.
    openPanel(trace());
    expect(rows()[1]).toContain('kvCacheHit');
    expect(rows()[3]).not.toContain('kvCache');
  });
});
