// Regression: the "sync" column merges one TaskQueue per account, and every
// queue numbers its tasks from 1. Two accounts syncing in the same second land
// in the merged history with the same id AND the same start time, so React
// logged "Encountered two children with the same key" on every Dashboard open.
// Task names embed the account id (`sync:account:{uuid}`), which is what tells
// them apart.

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
  it('renders merged per-account sync queues without a duplicate-key error', () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    const sameSecond = 1_789_135_641;
    const state: AllQueuesState = {
      ai: queue('ai', []),
      aiBackground: queue('aiBackground', []),
      db: queue('db', []),
      sync: {
        ...queue('sync', [entry(2, 'sync:account:aaaa', sameSecond), entry(2, 'sync:account:bbbb', sameSecond)]),
        running: [
          { id: 1, name: 'sync:account:aaaa', startedAt: sameSecond },
          { id: 1, name: 'sync:account:bbbb', startedAt: sameSecond },
        ],
      },
    };
    act(() => root.render(<QueuePanel state={state} accounts={[]} />));
    const dup = errors.mock.calls.filter((c) => c.some((a) => String(a).includes('same key')));
    expect(dup, 'duplicate-key errors').toHaveLength(0);
    expect(container.querySelectorAll('li').length).toBe(4);
  });
});
