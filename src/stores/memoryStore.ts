// Memory subsystem store — tasks + open threads.
//
// Source of truth is SQLite on the backend; this store caches the last
// fetched view plus the aggregate counts surfaced in the sidebar badge.
// Callers must destructure reactive fields (`const { tasks } = useMemoryStore()`)
// because `useMemoryStore.getState()` inside `useMemo` deps would not
// subscribe — see CLAUDE.md "Zustand Store Subscriptions".

import { listen } from '@tauri-apps/api/event';
import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type {
  CreatePendingTaskRequest,
  MemoryCountsSummary,
  MemoryFact,
  PendingTask,
  TaskCountsSummary,
  ThreadState,
} from '@/types';

interface MemoryStore {
  accountId: string | null;
  tasks: PendingTask[];
  openThreads: ThreadState[];
  counts: TaskCountsSummary;
  isLoadingTasks: boolean;
  isLoadingThreads: boolean;
  error: string | null;

  // Memory facts (inspector)
  facts: MemoryFact[];
  factCounts: MemoryCountsSummary;
  isLoadingFacts: boolean;
  factStatusFilter: 'all' | 'promoted' | 'candidate' | 'retired';

  loadForAccount: (accountId: string) => Promise<void>;
  refreshCounts: () => Promise<void>;
  refreshTasks: () => Promise<void>;
  refreshOpenThreads: () => Promise<void>;
  createTask: (req: CreatePendingTaskRequest) => Promise<PendingTask>;
  setTaskStatus: (taskId: string, status: string) => Promise<void>;

  // Memory facts actions
  loadFacts: (accountId: string) => Promise<void>;
  refreshFacts: () => Promise<void>;
  refreshFactCounts: () => Promise<void>;
  setFactStatusFilter: (filter: 'all' | 'promoted' | 'candidate' | 'retired') => Promise<void>;
  promoteFact: (factId: string) => Promise<void>;
  retireFact: (factId: string) => Promise<void>;
  updateFact: (factId: string, fact: string) => Promise<void>;
  deleteFact: (factId: string) => Promise<void>;

  reset: () => void;
}

const INITIAL_COUNTS: TaskCountsSummary = {
  totalOpen: 0,
  overdue: 0,
  dueToday: 0,
  awaitingThem: 0,
};

const INITIAL_FACT_COUNTS: MemoryCountsSummary = {
  total: 0,
  promoted: 0,
  candidate: 0,
};

export const useMemoryStore = create<MemoryStore>((set, get) => ({
  accountId: null,
  tasks: [],
  openThreads: [],
  counts: INITIAL_COUNTS,
  isLoadingTasks: false,
  isLoadingThreads: false,
  error: null,

  facts: [],
  factCounts: INITIAL_FACT_COUNTS,
  isLoadingFacts: false,
  factStatusFilter: 'all',

  loadForAccount: async (accountId) => {
    set({ accountId, isLoadingTasks: true, isLoadingThreads: true, error: null });
    try {
      const [tasks, openThreads, counts] = await Promise.all([
        api.listPendingTasks(accountId, { status: 'open' }),
        api.listOpenThreads(accountId, { awaiting: 'them' }),
        api.getTaskCounts(accountId),
      ]);
      // Guard against race: if the user switched accounts mid-flight, drop.
      if (get().accountId !== accountId) return;
      set({ tasks, openThreads, counts, isLoadingTasks: false, isLoadingThreads: false });
    } catch (e) {
      set({
        isLoadingTasks: false,
        isLoadingThreads: false,
        error: errorText(e),
      });
    }
  },

  refreshCounts: async () => {
    const accountId = get().accountId;
    if (!accountId) return;
    try {
      const counts = await api.getTaskCounts(accountId);
      if (get().accountId !== accountId) return;
      set({ counts });
    } catch (e) {
      set({ error: errorText(e) });
    }
  },

  refreshTasks: async () => {
    const accountId = get().accountId;
    if (!accountId) return;
    set({ isLoadingTasks: true });
    try {
      const tasks = await api.listPendingTasks(accountId, { status: 'open' });
      if (get().accountId !== accountId) return;
      set({ tasks, isLoadingTasks: false });
    } catch (e) {
      set({ isLoadingTasks: false, error: errorText(e) });
    }
  },

  refreshOpenThreads: async () => {
    const accountId = get().accountId;
    if (!accountId) return;
    set({ isLoadingThreads: true });
    try {
      const openThreads = await api.listOpenThreads(accountId, { awaiting: 'them' });
      if (get().accountId !== accountId) return;
      set({ openThreads, isLoadingThreads: false });
    } catch (e) {
      set({ isLoadingThreads: false, error: errorText(e) });
    }
  },

  createTask: async (req) => {
    const task = await api.createPendingTask(req);
    // Optimistically prepend to the local list and bump counts; a full refresh
    // is still triggered so ordering (priority/due_at) is correct.
    const tasks = [task, ...get().tasks];
    set({ tasks });
    void get().refreshTasks();
    void get().refreshCounts();
    return task;
  },

  setTaskStatus: async (taskId, status) => {
    // Optimistic update: drop the task from the "open" list immediately if the
    // new status is anything other than open. A refresh reconciles afterwards.
    const previous = get().tasks;
    const updated = status === 'open' ? previous : previous.filter((t) => t.id !== taskId);
    set({ tasks: updated });
    try {
      await api.updatePendingTaskStatus(taskId, status);
      void get().refreshCounts();
    } catch (e) {
      // Revert on failure.
      set({ tasks: previous, error: errorText(e) });
      throw e;
    }
  },

  loadFacts: async (accountId) => {
    set({ accountId, isLoadingFacts: true, error: null });
    try {
      const filter = get().factStatusFilter;
      const status = filter === 'all' ? undefined : filter;
      const [facts, factCounts] = await Promise.all([
        api.listMemoryFacts(accountId, { status }),
        api.getMemoryCounts(accountId),
      ]);
      if (get().accountId !== accountId) return;
      set({ facts, factCounts, isLoadingFacts: false });
    } catch (e) {
      set({ isLoadingFacts: false, error: errorText(e) });
    }
  },

  refreshFacts: async () => {
    const accountId = get().accountId;
    if (!accountId) return;
    set({ isLoadingFacts: true });
    try {
      const filter = get().factStatusFilter;
      const status = filter === 'all' ? undefined : filter;
      const facts = await api.listMemoryFacts(accountId, { status });
      if (get().accountId !== accountId) return;
      set({ facts, isLoadingFacts: false });
    } catch (e) {
      set({ isLoadingFacts: false, error: errorText(e) });
    }
  },

  refreshFactCounts: async () => {
    const accountId = get().accountId;
    if (!accountId) return;
    try {
      const factCounts = await api.getMemoryCounts(accountId);
      if (get().accountId !== accountId) return;
      set({ factCounts });
    } catch (e) {
      set({ error: errorText(e) });
    }
  },

  setFactStatusFilter: async (filter) => {
    set({ factStatusFilter: filter });
    await get().refreshFacts();
  },

  promoteFact: async (factId) => {
    const previous = get().facts;
    set({
      facts: previous.map((f) => (f.id === factId ? { ...f, status: 'promoted' } : f)),
    });
    try {
      await api.promoteMemoryFact(factId);
      void get().refreshFactCounts();
      // If filtering by 'candidate', the promoted row no longer belongs; refresh.
      if (get().factStatusFilter !== 'all' && get().factStatusFilter !== 'promoted') {
        void get().refreshFacts();
      }
    } catch (e) {
      set({ facts: previous, error: errorText(e) });
      throw e;
    }
  },

  retireFact: async (factId) => {
    const previous = get().facts;
    set({
      facts: previous.map((f) => (f.id === factId ? { ...f, status: 'retired' } : f)),
    });
    try {
      await api.retireMemoryFact(factId);
      void get().refreshFactCounts();
      if (get().factStatusFilter !== 'all' && get().factStatusFilter !== 'retired') {
        void get().refreshFacts();
      }
    } catch (e) {
      set({ facts: previous, error: errorText(e) });
      throw e;
    }
  },

  updateFact: async (factId, fact) => {
    const previous = get().facts;
    set({
      facts: previous.map((f) => (f.id === factId ? { ...f, fact } : f)),
    });
    try {
      await api.updateMemoryFact(factId, fact);
    } catch (e) {
      set({ facts: previous, error: errorText(e) });
      throw e;
    }
  },

  deleteFact: async (factId) => {
    const previous = get().facts;
    set({ facts: previous.filter((f) => f.id !== factId) });
    try {
      await api.deleteMemoryFact(factId);
      void get().refreshFactCounts();
    } catch (e) {
      set({ facts: previous, error: errorText(e) });
      throw e;
    }
  },

  reset: () =>
    set({
      accountId: null,
      tasks: [],
      openThreads: [],
      counts: INITIAL_COUNTS,
      isLoadingTasks: false,
      isLoadingThreads: false,
      error: null,
      facts: [],
      factCounts: INITIAL_FACT_COUNTS,
      isLoadingFacts: false,
      factStatusFilter: 'all',
    }),
}));

// Subscribe once (module-scope) to the backend `memory-facts-changed` event so
// the Memory inspector list and fact counts refresh automatically while the
// backfill is running. The backend emits this after each extract→embed→
// consolidate batch (see src-tauri/src/commands/memory.rs).
//
// We scope refreshes to the store's active account — events for other accounts
// are ignored. The memory store `refresh*` actions already guard against stale
// accountId mid-flight.
void listen<{ accountId?: string }>('memory-facts-changed', (event) => {
  const store = useMemoryStore.getState();
  const active = store.accountId;
  if (!active) return;
  const evtAccount = event.payload?.accountId;
  if (evtAccount && evtAccount !== active) return;
  void store.refreshFacts();
  void store.refreshFactCounts();
});
