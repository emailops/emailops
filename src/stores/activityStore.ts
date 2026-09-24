// Long-running AI work for the status bar: what runs (the task-queue
// snapshot, polled) and how far along it is (each process's status API or
// progress event). See `lib/backgroundActivity`.

import { create } from 'zustand';
import * as api from '@/lib/api';
import {
  type Activity,
  type ActivityKind,
  type ActivityProgress,
  progressKey,
  summarizeActivities,
} from '@/lib/backgroundActivity';

interface ActivityStore {
  activities: Activity[];
  /** Latest known progress per process (`progressKey`). */
  progress: Record<string, ActivityProgress>;
  polling: boolean;
  /** Record progress a process reported by event. `null` clears it. */
  setProgress: (kind: ActivityKind, target: string | null, progress: ActivityProgress | null) => void;
  /** Re-read the queues and the status of whatever runs. */
  refresh: () => Promise<void>;
}

/** Ask a running process for its numbers, when it has a status API. */
async function fetchProgress(a: Activity): Promise<ActivityProgress | null> {
  if (!a.target) return null;
  switch (a.kind) {
    case 'lensBackfill':
    case 'lensRun': {
      const s = await api.getLensStatus(a.target);
      return s.total > 0 ? { current: s.processed, total: s.total } : null;
    }
    case 'memoryBackfill': {
      const s = await api.getMemoryBackfillStatus(a.target);
      return { current: s.remaining, total: null, unit: 'remaining' };
    }
    case 'tasksBackfill': {
      const s = await api.getTaskBackfillStatus(a.target);
      return { current: s.remaining, total: null, unit: 'remaining' };
    }
    default:
      return null;
  }
}

export const useActivityStore = create<ActivityStore>((set, get) => ({
  activities: [],
  progress: {},
  polling: false,

  setProgress: (kind, target, progress) => {
    const key = progressKey(kind, target);
    set((s) => {
      const next = { ...s.progress };
      if (progress) next[key] = progress;
      else delete next[key];
      return {
        progress: next,
        activities: s.activities.map((a) => (a.key === key ? { ...a, progress } : a)),
      };
    });
  },

  refresh: async () => {
    if (get().polling) return;
    set({ polling: true });
    try {
      const queues = await api.getQueueState();
      const polled = { ...get().progress };
      const running = summarizeActivities(queues, polled);
      await Promise.all(
        running.map(async (a) => {
          try {
            const p = await fetchProgress(a);
            if (p) polled[a.key] = p;
          } catch {
            // A status call failing only costs the numbers, not the entry.
          }
        }),
      );
      // Forget the progress of what stopped running.
      const live = new Set(running.map((a) => a.key));
      for (const key of Object.keys(polled)) if (!live.has(key)) delete polled[key];
      set({ progress: polled, activities: summarizeActivities(queues, polled) });
    } catch {
      // The status bar is best-effort: keep the last known state.
    } finally {
      set({ polling: false });
    }
  },
}));
