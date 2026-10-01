// activityStore: the status bar's view of long-running AI work.
//
// `refresh` polls the task queues, asks each running process with a status API
// for its numbers, forgets the numbers of what stopped, and must never let a
// failed call wipe the bar or leave `polling` stuck on (which would stop every
// later refresh).

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AllQueuesState, QueueStateSnapshot } from '@/types';

const api = vi.hoisted(() => ({
  getQueueState: vi.fn(),
  getLensStatus: vi.fn(),
  getMemoryBackfillStatus: vi.fn(),
  getTaskBackfillStatus: vi.fn(),
}));

vi.mock('@/lib/api', () => api);

import { useActivityStore } from './activityStore';

function queue(name: string, running: string[]): QueueStateSnapshot {
  return {
    name,
    concurrency: 1,
    running: running.map((n, i) => ({ id: i, name: n, startedAt: 0 })),
    pending: [],
    history: [],
  };
}

function queues(aiBackground: string[], db: string[] = []): AllQueuesState {
  return {
    ai: queue('ai', []),
    aiBackground: queue('ai-bg', aiBackground),
    db: queue('db', db),
    sync: queue('sync', []),
  };
}

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  useActivityStore.setState({ activities: [], progress: {}, polling: false });
});

describe('activityStore.refresh', () => {
  it('shows each running process with the numbers its status API reports', async () => {
    api.getQueueState.mockResolvedValue(
      queues(['lens:backfill:lens-1', 'memory:backfill:acc-1', 'tasks:backfill:acc-1', 'junk:backfill:acc-1']),
    );
    api.getLensStatus.mockResolvedValue({ processed: 3, total: 10 });
    api.getMemoryBackfillStatus.mockResolvedValue({ remaining: 40 });
    api.getTaskBackfillStatus.mockResolvedValue({ remaining: 7 });

    await useActivityStore.getState().refresh();

    const s = useActivityStore.getState();
    expect(s.activities.map((a) => [a.key, a.progress])).toEqual([
      ['lensBackfill:lens-1', { current: 3, total: 10 }],
      ['memoryBackfill:acc-1', { current: 40, total: null, unit: 'remaining' }],
      ['tasksBackfill:acc-1', { current: 7, total: null, unit: 'remaining' }],
      ['junk:acc-1', null],
    ]);
    expect(s.polling).toBe(false);
  });

  it('shows no numbers for a Lens run that has not counted its emails yet', async () => {
    api.getQueueState.mockResolvedValue(queues(['lens:single:lens-1']));
    api.getLensStatus.mockResolvedValue({ processed: 0, total: 0 });

    await useActivityStore.getState().refresh();

    expect(useActivityStore.getState().activities[0].progress).toBeNull();
  });

  it('keeps an entry whose status call failed, without numbers', async () => {
    api.getQueueState.mockResolvedValue(queues(['lens:backfill:lens-1']));
    api.getLensStatus.mockRejectedValue(new Error('gone'));

    await useActivityStore.getState().refresh();

    expect(useActivityStore.getState().activities.map((a) => a.key)).toEqual(['lensBackfill:lens-1']);
  });

  it('forgets the progress of a process that is no longer running', async () => {
    useActivityStore.setState({
      progress: { 'lensBackfill:old': { current: 1, total: 2 }, embeddings: { current: 5, total: 9 } },
    });
    api.getQueueState.mockResolvedValue(queues([], ['embeddings:generate:all']));

    await useActivityStore.getState().refresh();

    const s = useActivityStore.getState();
    expect(s.progress).toEqual({ embeddings: { current: 5, total: 9 } });
    expect(s.activities[0].progress).toEqual({ current: 5, total: 9 });
  });

  it('keeps the last known state and stops polling when the queues cannot be read', async () => {
    const shown = [{ key: 'junk:acc-1', kind: 'junk' as const, target: 'acc-1', progress: null }];
    useActivityStore.setState({ activities: shown });
    api.getQueueState.mockRejectedValue(new Error('backend restarting'));

    await useActivityStore.getState().refresh();

    expect(useActivityStore.getState().activities).toEqual(shown);
    expect(useActivityStore.getState().polling).toBe(false);
  });

  it('does not start a second poll while one is in flight', async () => {
    useActivityStore.setState({ polling: true });

    await useActivityStore.getState().refresh();

    expect(api.getQueueState).not.toHaveBeenCalled();
  });
});

describe('activityStore.setProgress', () => {
  it('records event-reported progress on the matching entry and clears it with null', () => {
    useActivityStore.setState({
      activities: [
        { key: 'classification', kind: 'classification', target: 'acc-1', progress: null },
        { key: 'junk:acc-1', kind: 'junk', target: 'acc-1', progress: null },
      ],
    });

    useActivityStore.getState().setProgress('classification', 'acc-1', { current: 2, total: 8 });
    let s = useActivityStore.getState();
    expect(s.progress).toEqual({ classification: { current: 2, total: 8 } });
    expect(s.activities.map((a) => a.progress)).toEqual([{ current: 2, total: 8 }, null]);

    useActivityStore.getState().setProgress('classification', 'acc-1', null);
    s = useActivityStore.getState();
    expect(s.progress).toEqual({});
    expect(s.activities[0].progress).toBeNull();
  });
});
