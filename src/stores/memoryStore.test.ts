// memoryStore: tasks, open threads and memory facts for the active account.
//
// Every action writes optimistically and must restore what it overwrote when
// the backend refuses, and every load must drop an answer that arrives after
// the user switched accounts. These tests pin both, plus the module-level
// `memory-facts-changed` subscription that refreshes the inspector.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { MemoryFact, PendingTask, TaskCountsSummary, ThreadState } from '@/types';

type FactsChanged = (event: { payload?: { accountId?: string } }) => void;
// Hoisted with the mock: the store subscribes while its module is imported.
const subscription = vi.hoisted(() => ({ handler: null as FactsChanged | null }));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((_name: string, handler: FactsChanged) => {
    subscription.handler = handler;
    return Promise.resolve(() => {});
  }),
}));

const api = vi.hoisted(() => ({
  listPendingTasks: vi.fn(),
  listOpenThreads: vi.fn(),
  getTaskCounts: vi.fn(),
  createPendingTask: vi.fn(),
  updatePendingTaskStatus: vi.fn(),
  listMemoryFacts: vi.fn(),
  getMemoryCounts: vi.fn(),
  promoteMemoryFact: vi.fn(),
  retireMemoryFact: vi.fn(),
  updateMemoryFact: vi.fn(),
  deleteMemoryFact: vi.fn(),
}));

vi.mock('@/lib/api', () => api);

import { useMemoryStore } from './memoryStore';

function task(id: string): PendingTask {
  return {
    id,
    accountId: 'acc-1',
    title: `Task ${id}`,
    detail: null,
    source: 'user',
    sourceEmailId: null,
    sourceThreadId: null,
    assignee: 'me',
    status: 'open',
    priority: 'normal',
    dueAt: null,
    completedAt: null,
    company: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

function thread(threadId: string): ThreadState {
  return {
    accountId: 'acc-1',
    threadId,
    awaiting: 'them',
    lastInboundAt: null,
    lastOutboundAt: null,
    lastTouchedAt: 0,
    summary: null,
    commitment: null,
    deadlineAt: null,
    participants: [],
    updatedAt: 0,
  };
}

function fact(id: string, status = 'candidate'): MemoryFact {
  return {
    id,
    accountId: 'acc-1',
    subjectKind: 'user',
    subjectKey: 'me',
    fact: `Fact ${id}`,
    source: 'extraction',
    sourceEmailId: null,
    confidence: 0.8,
    score: 1,
    status,
    lastUsedAt: null,
    domain: null,
    vigency: null,
    company: null,
    createdAt: 0,
    updatedAt: 0,
  };
}

const COUNTS: TaskCountsSummary = { totalOpen: 2, overdue: 1, dueToday: 0, awaitingThem: 1 };

/** A promise the test resolves by hand, to interleave two loads. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

/** Let the fire-and-forget refreshes (`void get().refresh…()`) settle. */
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.listPendingTasks.mockResolvedValue([]);
  api.listOpenThreads.mockResolvedValue([]);
  api.getTaskCounts.mockResolvedValue(COUNTS);
  api.listMemoryFacts.mockResolvedValue([]);
  api.getMemoryCounts.mockResolvedValue({ total: 0, promoted: 0, candidate: 0 });
  for (const fn of [api.updatePendingTaskStatus, api.promoteMemoryFact, api.retireMemoryFact]) {
    fn.mockResolvedValue(undefined);
  }
  api.updateMemoryFact.mockResolvedValue(undefined);
  api.deleteMemoryFact.mockResolvedValue(undefined);
  useMemoryStore.getState().reset();
});

describe('memoryStore.loadForAccount', () => {
  it('loads open tasks, threads awaiting the other side, and the counts', async () => {
    api.listPendingTasks.mockResolvedValue([task('t1')]);
    api.listOpenThreads.mockResolvedValue([thread('th1')]);

    await useMemoryStore.getState().loadForAccount('acc-1');

    const s = useMemoryStore.getState();
    expect(api.listPendingTasks).toHaveBeenCalledWith('acc-1', { status: 'open' });
    expect(api.listOpenThreads).toHaveBeenCalledWith('acc-1', { awaiting: 'them' });
    expect(s.tasks.map((t) => t.id)).toEqual(['t1']);
    expect(s.openThreads.map((t) => t.threadId)).toEqual(['th1']);
    expect(s.counts).toEqual(COUNTS);
    expect(s.isLoadingTasks).toBe(false);
    expect(s.isLoadingThreads).toBe(false);
  });

  it('drops the answer for an account the user already switched away from', async () => {
    const slow = deferred<PendingTask[]>();
    api.listPendingTasks.mockReturnValueOnce(slow.promise).mockResolvedValueOnce([task('b-task')]);

    const first = useMemoryStore.getState().loadForAccount('acc-a');
    await useMemoryStore.getState().loadForAccount('acc-b');
    slow.resolve([task('a-task')]);
    await first;

    const s = useMemoryStore.getState();
    expect(s.accountId).toBe('acc-b');
    expect(s.tasks.map((t) => t.id)).toEqual(['b-task']);
  });

  it('clears both loading flags and keeps the error when a request fails', async () => {
    api.getTaskCounts.mockRejectedValue(new Error('db locked'));

    await useMemoryStore.getState().loadForAccount('acc-1');

    const s = useMemoryStore.getState();
    expect(s.isLoadingTasks).toBe(false);
    expect(s.isLoadingThreads).toBe(false);
    expect(s.error).toContain('db locked');
  });
});

describe('memoryStore refreshes', () => {
  it('do nothing before an account is loaded', async () => {
    const s = useMemoryStore.getState();
    await Promise.all([
      s.refreshCounts(),
      s.refreshTasks(),
      s.refreshOpenThreads(),
      s.refreshFacts(),
      s.refreshFactCounts(),
    ]);

    expect(api.getTaskCounts).not.toHaveBeenCalled();
    expect(api.listPendingTasks).not.toHaveBeenCalled();
    expect(api.listOpenThreads).not.toHaveBeenCalled();
    expect(api.listMemoryFacts).not.toHaveBeenCalled();
    expect(api.getMemoryCounts).not.toHaveBeenCalled();
  });

  it('record a failed task refresh without leaving the list spinning', async () => {
    await useMemoryStore.getState().loadForAccount('acc-1');
    api.listPendingTasks.mockRejectedValue(new Error('offline'));

    await useMemoryStore.getState().refreshTasks();

    expect(useMemoryStore.getState().isLoadingTasks).toBe(false);
    expect(useMemoryStore.getState().error).toContain('offline');
  });
});

describe('memoryStore tasks', () => {
  it('shows a created task at the top at once and then refreshes the list and counts', async () => {
    await useMemoryStore.getState().loadForAccount('acc-1');
    useMemoryStore.setState({ tasks: [task('old')] });
    api.createPendingTask.mockResolvedValue(task('new'));
    api.listPendingTasks.mockClear();
    api.getTaskCounts.mockClear();
    api.listPendingTasks.mockReturnValue(new Promise(() => {}));

    const created = await useMemoryStore.getState().createTask({ accountId: 'acc-1', title: 'Task new' });

    expect(created.id).toBe('new');
    expect(useMemoryStore.getState().tasks.map((t) => t.id)).toEqual(['new', 'old']);
    expect(api.listPendingTasks).toHaveBeenCalledTimes(1);
    expect(api.getTaskCounts).toHaveBeenCalledTimes(1);
  });

  it('removes a task closed from the open list before the backend answers', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', tasks: [task('t1'), task('t2')] });
    const pending = deferred<void>();
    api.updatePendingTaskStatus.mockReturnValue(pending.promise);

    const done = useMemoryStore.getState().setTaskStatus('t1', 'done');

    expect(useMemoryStore.getState().tasks.map((t) => t.id)).toEqual(['t2']);
    pending.resolve();
    await done;
    expect(api.updatePendingTaskStatus).toHaveBeenCalledWith('acc-1', 't1', 'done');
  });

  it('keeps a task that is set back to open in the list', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', tasks: [task('t1')] });

    await useMemoryStore.getState().setTaskStatus('t1', 'open');

    expect(useMemoryStore.getState().tasks.map((t) => t.id)).toEqual(['t1']);
  });

  it('puts the task back and rethrows when the status change is refused', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', tasks: [task('t1'), task('t2')] });
    api.updatePendingTaskStatus.mockRejectedValue(new Error('refused'));

    await expect(useMemoryStore.getState().setTaskStatus('t1', 'dismissed')).rejects.toThrow('refused');

    expect(useMemoryStore.getState().tasks.map((t) => t.id)).toEqual(['t1', 't2']);
    expect(useMemoryStore.getState().error).toContain('refused');
  });
});

describe('memoryStore facts', () => {
  it('asks for every status under the "all" filter and for one status otherwise', async () => {
    await useMemoryStore.getState().loadFacts('acc-1');
    expect(api.listMemoryFacts).toHaveBeenLastCalledWith('acc-1', { status: undefined });

    await useMemoryStore.getState().setFactStatusFilter('candidate');

    expect(useMemoryStore.getState().factStatusFilter).toBe('candidate');
    expect(api.listMemoryFacts).toHaveBeenLastCalledWith('acc-1', { status: 'candidate' });
  });

  it('drops facts that arrive for an account the user already left', async () => {
    const slow = deferred<MemoryFact[]>();
    api.listMemoryFacts.mockReturnValueOnce(slow.promise).mockResolvedValueOnce([fact('b')]);

    const first = useMemoryStore.getState().loadFacts('acc-a');
    await useMemoryStore.getState().loadFacts('acc-b');
    slow.resolve([fact('a')]);
    await first;

    expect(useMemoryStore.getState().facts.map((f) => f.id)).toEqual(['b']);
  });

  it('promotes a fact in place and refetches the list when it no longer matches the filter', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', facts: [fact('f1')], factStatusFilter: 'candidate' });
    // Keep the refetch in flight so the optimistic row is still what the store holds.
    api.listMemoryFacts.mockReturnValue(new Promise(() => {}));

    await useMemoryStore.getState().promoteFact('f1');
    expect(useMemoryStore.getState().facts[0].status).toBe('promoted');
    await settle();

    expect(api.promoteMemoryFact).toHaveBeenCalledWith('acc-1', 'f1');
    expect(api.getMemoryCounts).toHaveBeenCalled();
    expect(api.listMemoryFacts).toHaveBeenCalledWith('acc-1', { status: 'candidate' });
  });

  it('does not refetch the list after promoting under the "all" or "promoted" filter', async () => {
    for (const filter of ['all', 'promoted'] as const) {
      api.listMemoryFacts.mockClear();
      useMemoryStore.setState({ accountId: 'acc-1', facts: [fact('f1')], factStatusFilter: filter });

      await useMemoryStore.getState().promoteFact('f1');
      await settle();

      expect(api.listMemoryFacts).not.toHaveBeenCalled();
    }
  });

  it('retires a fact in place and refetches only when the filter would now hide it', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', facts: [fact('f1', 'promoted')], factStatusFilter: 'retired' });
    await useMemoryStore.getState().retireFact('f1');
    await settle();
    expect(useMemoryStore.getState().facts[0].status).toBe('retired');
    expect(api.listMemoryFacts).not.toHaveBeenCalled();

    useMemoryStore.setState({ facts: [fact('f2', 'promoted')], factStatusFilter: 'promoted' });
    await useMemoryStore.getState().retireFact('f2');
    await settle();
    expect(api.listMemoryFacts).toHaveBeenCalledWith('acc-1', { status: 'promoted' });
  });

  it.each([
    ['promoteFact', 'promoteMemoryFact'],
    ['retireFact', 'retireMemoryFact'],
    ['deleteFact', 'deleteMemoryFact'],
  ] as const)('%s restores the facts it changed and rethrows when refused', async (action, call) => {
    const before = [fact('f1'), fact('f2')];
    useMemoryStore.setState({ accountId: 'acc-1', facts: before });
    api[call].mockRejectedValue(new Error('refused'));

    await expect(useMemoryStore.getState()[action]('f1')).rejects.toThrow('refused');

    expect(useMemoryStore.getState().facts).toEqual(before);
    expect(useMemoryStore.getState().error).toContain('refused');
  });

  it('edits a fact in place and restores the old text when the edit is refused', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', facts: [fact('f1')] });
    await useMemoryStore.getState().updateFact('f1', 'Prefers morning meetings');
    expect(useMemoryStore.getState().facts[0].fact).toBe('Prefers morning meetings');
    expect(api.updateMemoryFact).toHaveBeenCalledWith('acc-1', 'f1', 'Prefers morning meetings');

    api.updateMemoryFact.mockRejectedValue(new Error('refused'));
    await expect(useMemoryStore.getState().updateFact('f1', 'Other text')).rejects.toThrow('refused');
    expect(useMemoryStore.getState().facts[0].fact).toBe('Prefers morning meetings');
  });

  it('removes a deleted fact at once and refreshes the counts', async () => {
    useMemoryStore.setState({ accountId: 'acc-1', facts: [fact('f1'), fact('f2')] });

    await useMemoryStore.getState().deleteFact('f1');
    await settle();

    expect(useMemoryStore.getState().facts.map((f) => f.id)).toEqual(['f2']);
    expect(api.getMemoryCounts).toHaveBeenCalledWith('acc-1');
  });
});

describe('memory-facts-changed event', () => {
  function emit(accountId?: string) {
    if (!subscription.handler) throw new Error('store never subscribed to memory-facts-changed');
    subscription.handler({ payload: accountId === undefined ? {} : { accountId } });
  }

  it('refreshes the facts and counts of the active account', async () => {
    useMemoryStore.setState({ accountId: 'acc-1' });

    emit('acc-1');
    await settle();

    expect(api.listMemoryFacts).toHaveBeenCalledWith('acc-1', { status: undefined });
    expect(api.getMemoryCounts).toHaveBeenCalledWith('acc-1');
  });

  it('ignores an event for another account, or while no account is loaded', async () => {
    useMemoryStore.setState({ accountId: 'acc-1' });
    emit('acc-2');
    useMemoryStore.setState({ accountId: null });
    emit('acc-1');
    await settle();

    expect(api.listMemoryFacts).not.toHaveBeenCalled();
    expect(api.getMemoryCounts).not.toHaveBeenCalled();
  });

  it('treats an event without an account as one for the active account', async () => {
    useMemoryStore.setState({ accountId: 'acc-1' });

    emit();
    await settle();

    expect(api.getMemoryCounts).toHaveBeenCalledWith('acc-1');
  });
});
