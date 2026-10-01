/**
 * The undo window behind archive and delete.
 *
 * An action's visible effect is applied at once by the caller; what reaches
 * the provider (`commit`) is held back for `windowMs`. Undo inside the window
 * calls `undo` and the provider never hears of it, so no provider needs an
 * "un-trash" or "un-archive" path. The window closes — and the action commits —
 * when the timer fires, when another action is scheduled (one undo at a time,
 * as Gmail does), or on `flushAll` (the app is about to unload).
 *
 * Pure of React and of the stores: the timer is the only side effect, so
 * vitest's fake timers drive it.
 */

export interface PendingAction {
  /** Send the action to the provider. Errors go to `onError`; the commit is
   *  responsible for its own rollback. */
  commit: () => Promise<void>;
  /** Put back what the caller changed optimistically. */
  undo: () => void;
}

export type PendingOutcome = 'committed' | 'undone';

export interface PendingActionQueue {
  /** Hold `action` for the window; commits whatever was pending before. */
  schedule: (action: PendingAction) => number;
  /** Undo a pending action. False when it already committed (or never was). */
  undo: (id: number) => boolean;
  /** Commit a pending action now. */
  flush: (id: number) => Promise<void>;
  /** Commit everything pending now. */
  flushAll: () => Promise<void>;
  /** Resolves once the action committed or was undone; null for unknown ids. */
  settled: (id: number) => Promise<PendingOutcome | null>;
  pendingCount: () => number;
}

interface Entry {
  action: PendingAction;
  timer: ReturnType<typeof setTimeout>;
  outcome: Promise<PendingOutcome>;
  resolve: (outcome: PendingOutcome) => void;
}

export function createPendingActionQueue(options: {
  windowMs: number;
  onError?: (error: unknown) => void;
}): PendingActionQueue {
  const pending = new Map<number, Entry>();
  let nextId = 1;

  const commit = async (id: number): Promise<void> => {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    clearTimeout(entry.timer);
    try {
      await entry.action.commit();
    } catch (error) {
      options.onError?.(error);
    }
    entry.resolve('committed');
  };

  const flushAll = async (): Promise<void> => {
    await Promise.all([...pending.keys()].map(commit));
  };

  return {
    schedule: (action) => {
      void flushAll();
      const id = nextId++;
      let resolve: (outcome: PendingOutcome) => void = () => {};
      const outcome = new Promise<PendingOutcome>((r) => {
        resolve = r;
      });
      const timer = setTimeout(() => void commit(id), options.windowMs);
      pending.set(id, { action, timer, outcome, resolve });
      return id;
    },
    undo: (id) => {
      const entry = pending.get(id);
      if (!entry) return false;
      pending.delete(id);
      clearTimeout(entry.timer);
      entry.action.undo();
      entry.resolve('undone');
      return true;
    },
    flush: commit,
    flushAll,
    settled: (id) => pending.get(id)?.outcome ?? Promise.resolve(null),
    pendingCount: () => pending.size,
  };
}
