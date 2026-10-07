// Scheduling helpers for refreshes that sync events can fire far faster than
// the backend can serve them — several accounts syncing at once in the
// All-accounts view each emit their own batch and completion events.

/**
 * Serialises `run` and folds every request made while a run is in flight into
 * one follow-up run over the distinct keys requested meanwhile. The promise a
 * request returns settles with the run that covers its key.
 */
export function createKeyedCoalescer<K>(run: (keys: K[]) => Promise<void>): (key: K) => Promise<void> {
  let running = false;
  let pending: { keys: Set<K>; done: Promise<void>; settle: (err?: unknown) => void } | null = null;

  const start = (keys: K[]): Promise<void> => {
    running = true;
    return run(keys).finally(() => {
      running = false;
      if (pending) {
        const next = pending;
        pending = null;
        start([...next.keys]).then(
          () => next.settle(),
          (err) => next.settle(err ?? new Error('refresh failed')),
        );
      }
    });
  };

  return (key) => {
    if (!running) return start([key]);
    if (!pending) {
      let resolve!: () => void;
      let reject!: (err: unknown) => void;
      const done = new Promise<void>((res, rej) => {
        resolve = res;
        reject = rej;
      });
      pending = { keys: new Set(), done, settle: (err) => (err === undefined ? resolve() : reject(err)) };
    }
    pending.keys.add(key);
    return pending.done;
  };
}

export interface Throttle {
  /** Run now if the window is quiet, otherwise once when it ends. */
  call: () => void;
  /** Run now and drop any pending trailing run. */
  flush: () => void;
  /** Drop any pending trailing run. */
  cancel: () => void;
}

/** Leading + trailing throttle: at most one run per `intervalMs`, never losing the last call. */
export function createThrottle(fn: () => void, intervalMs: number): Throttle {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let trailing = false;

  const openWindow = () => {
    timer = setTimeout(() => {
      timer = null;
      if (trailing) {
        trailing = false;
        fn();
        openWindow();
      }
    }, intervalMs);
  };

  const cancel = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    trailing = false;
  };

  return {
    call: () => {
      if (timer !== null) {
        trailing = true;
        return;
      }
      fn();
      openWindow();
    },
    flush: () => {
      cancel();
      fn();
      openWindow();
    },
    cancel,
  };
}
