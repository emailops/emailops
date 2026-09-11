// Regression: a task that is retried (the demo accounts fail auth on every
// sync) finishes more than once and lands in one queue's history twice with the
// same task id, so React logged "Encountered two children with the same key,
// `3`" on every Dashboard open. The key must include the attempt, not only the id.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AllQueuesState, QueueStateSnapshot, TaskHistoryEntry } from '@/types';
import { QueuePanel } from './QueuePanel';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function entry(id: number, name: string, startedAt: number): TaskHistoryEntry {
  return { id, name, startedAt, finishedAt: startedAt + 1, durationSecs: 1, status: 'ok' };
}
function queue(name: string, history: TaskHistoryEntry[]): QueueStateSnapshot {
  return { name, concurrency: 1, running: [], pending: [], history };
}

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
  vi.restoreAllMocks();
});

describe('QueuePanel history keys', () => {
  it('renders a retried task twice in one history without a duplicate-key error', () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    const state: AllQueuesState = {
      ai: queue('ai', []),
      aiBackground: queue('aiBackground', []),
      db: queue('db', []),
      sync: queue('sync', [entry(4, 'sync', 120), entry(3, 'sync', 110), entry(4, 'sync', 100), entry(3, 'sync', 90)]),
    };
    act(() => root.render(<QueuePanel state={state} accounts={[]} />));
    const dup = errors.mock.calls.filter((c) => c.some((a) => String(a).includes('same key')));
    expect(dup, 'duplicate-key errors').toHaveLength(0);
    expect(container.querySelectorAll('li').length).toBe(4);
  });
});
