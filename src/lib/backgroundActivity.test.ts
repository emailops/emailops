import { describe, expect, it } from 'vitest';
import type { AllQueuesState, QueueStateSnapshot, TaskInfo } from '@/types';
import { classifyTask, summarizeActivities } from './backgroundActivity';

function queue(name: string, running: string[], pending: string[] = []): QueueStateSnapshot {
  const task = (n: string, i: number): TaskInfo => ({ id: i, name: n, startedAt: 0 });
  return { name, concurrency: 2, running: running.map(task), pending: pending.map(task), history: [] };
}

function queues(ai: string[], aiBackground: string[], db: string[] = []): AllQueuesState {
  return {
    ai: queue('ai', ai),
    aiBackground: queue('ai-bg', aiBackground),
    db: queue('db', db),
    sync: queue('sync', []),
  };
}

describe('classifyTask', () => {
  it.each([
    ['lens:backfill:lens-1', 'lensBackfill', 'lens-1'],
    ['lens:single:lens-1', 'lensRun', 'lens-1'],
    ['lens:incremental:lens-2', 'lensRun', 'lens-2'],
    ['memory:backfill:acct-1', 'memoryBackfill', 'acct-1'],
    ['memory:extract+consolidate:acct-1:sync-3', 'memoryExtract', 'acct-1'],
    ['tasks:backfill:acct-1', 'tasksBackfill', 'acct-1'],
    ['tasks:extract:acct-1:sync-3', 'tasksExtract', 'acct-1'],
    ['classify:account:acct-1', 'classification', 'acct-1'],
    ['classify:new_emails:acct-1:sync-3', 'classification', 'acct-1'],
    ['reclassify:rule_update:rule-9', 'classification', 'rule-9'],
    ['embeddings:generate:all', 'embeddings', 'all'],
    ['embeddings:after_sync:acct-1:sync-3', 'embeddings', 'acct-1'],
    ['junk:backfill:acct-1', 'junk', 'acct-1'],
    ['model_download:qwen3.5-4b', 'modelDownload', 'qwen3.5-4b'],
  ])('%s is an expensive AI process', (name, kind, target) => {
    expect(classifyTask(name)).toEqual({ kind, target });
  });

  it.each([
    'chat:turn:conv-1',
    'chat:prewarm',
    'draft:email-1',
    'translate:email-1',
    'detect-lang:email-1',
    'sync:acct-1',
    'model_link:x',
  ])('%s is not shown', (name) => {
    expect(classifyTask(name)).toBeNull();
  });
});

describe('summarizeActivities', () => {
  it('lists running expensive processes with their progress, skipping the rest', () => {
    const acts = summarizeActivities(
      queues(['chat:turn:c1', 'draft:e1'], ['lens:backfill:lens-1', 'embeddings:generate:all'], ['model_download:m1']),
      {
        'lensBackfill:lens-1': { current: 120, total: 400 },
        embeddings: { current: 30, total: 90 },
        'modelDownload:m1': { current: 50, total: 100, unit: 'bytes' },
      },
    );
    expect(acts.map((a) => [a.kind, a.target, a.progress])).toEqual([
      ['lensBackfill', 'lens-1', { current: 120, total: 400 }],
      ['embeddings', 'all', { current: 30, total: 90 }],
      ['modelDownload', 'm1', { current: 50, total: 100, unit: 'bytes' }],
    ]);
  });

  it('shows one entry per process even when several tasks of it run', () => {
    const acts = summarizeActivities(
      queues([], ['classify:new_emails:a1:s1', 'classify:new_emails:a1:s2', 'junk:score:a1:s1']),
      {},
    );
    expect(acts.map((a) => a.kind)).toEqual(['classification', 'junk']);
    expect(acts[0].progress).toBeNull();
  });

  it('ignores queued work: only what runs now', () => {
    const q = queues([], []);
    q.aiBackground.pending = [{ id: 1, name: 'lens:backfill:lens-1', startedAt: 0 }];
    expect(summarizeActivities(q, {})).toEqual([]);
  });
});
