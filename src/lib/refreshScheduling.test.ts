import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createKeyedCoalescer, createThrottle } from './refreshScheduling';

/** A promise the test resolves or rejects by hand. */
function deferred() {
  let resolve!: () => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<void>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('createKeyedCoalescer', () => {
  it('runs a request at once when idle', async () => {
    const run = vi.fn(async (_keys: string[]) => {});
    const request = createKeyedCoalescer(run);

    await request('acc-1');

    expect(run).toHaveBeenCalledWith(['acc-1']);
  });

  it('folds requests made during a run into one follow-up run', async () => {
    const first = deferred();
    const run = vi.fn((_keys: string[]) => (run.mock.calls.length === 1 ? first.promise : Promise.resolve()));
    const request = createKeyedCoalescer(run);

    const p1 = request('acc-1');
    const p2 = request('acc-2');
    const p3 = request('acc-3');
    first.resolve();
    await Promise.all([p1, p2, p3]);

    expect(run.mock.calls).toEqual([[['acc-1']], [['acc-2', 'acc-3']]]);
  });

  it('drops a key repeated while it waits', async () => {
    const first = deferred();
    const run = vi.fn((_keys: string[]) => (run.mock.calls.length === 1 ? first.promise : Promise.resolve()));
    const request = createKeyedCoalescer(run);

    const p1 = request('acc-1');
    const p2 = request('acc-2');
    const p3 = request('acc-2');
    first.resolve();
    await Promise.all([p1, p2, p3]);

    expect(run.mock.calls[1]).toEqual([['acc-2']]);
  });

  it('rejects the callers of a failed run and keeps serving later requests', async () => {
    const run = vi.fn(async (_keys: string[]) => {
      if (run.mock.calls.length === 1) throw new Error('db busy');
    });
    const request = createKeyedCoalescer(run);

    await expect(request('acc-1')).rejects.toThrow('db busy');
    await request('acc-2');

    expect(run).toHaveBeenLastCalledWith(['acc-2']);
  });
});

describe('createThrottle', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('runs the first call immediately', () => {
    const fn = vi.fn();
    const throttle = createThrottle(fn, 1000);

    throttle.call();

    expect(fn).toHaveBeenCalledTimes(1);
  });

  it('collapses calls inside the window into one trailing run', () => {
    const fn = vi.fn();
    const throttle = createThrottle(fn, 1000);

    throttle.call();
    throttle.call();
    throttle.call();
    vi.advanceTimersByTime(999);
    expect(fn).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);

    expect(fn).toHaveBeenCalledTimes(2);
  });

  it('runs at once again after a quiet window', () => {
    const fn = vi.fn();
    const throttle = createThrottle(fn, 1000);

    throttle.call();
    vi.advanceTimersByTime(1000);
    throttle.call();

    expect(fn).toHaveBeenCalledTimes(2);
  });

  it('flush runs now and drops the pending trailing run', () => {
    const fn = vi.fn();
    const throttle = createThrottle(fn, 1000);

    throttle.call();
    throttle.call();
    throttle.flush();
    vi.advanceTimersByTime(5000);

    expect(fn).toHaveBeenCalledTimes(2);
  });

  it('cancel drops the pending trailing run', () => {
    const fn = vi.fn();
    const throttle = createThrottle(fn, 1000);

    throttle.call();
    throttle.call();
    throttle.cancel();
    vi.advanceTimersByTime(5000);

    expect(fn).toHaveBeenCalledTimes(1);
  });
});
