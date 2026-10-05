// The undo window behind archive and delete: an action is held back for a few
// seconds, then committed; undo drops it without ever reaching the provider.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createPendingActionQueue } from './pendingActions';

function action() {
  return { commit: vi.fn(async () => undefined), undo: vi.fn() };
}

describe('createPendingActionQueue', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('commits an action once the window closes', () => {
    const queue = createPendingActionQueue({ windowMs: 6000 });
    const a = action();
    queue.schedule(a);

    vi.advanceTimersByTime(5999);
    expect(a.commit).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(a.commit).toHaveBeenCalledTimes(1);
    expect(a.undo).not.toHaveBeenCalled();
    expect(queue.pendingCount()).toBe(0);
  });

  it('undo restores and never commits', () => {
    const queue = createPendingActionQueue({ windowMs: 6000 });
    const a = action();
    const id = queue.schedule(a);

    expect(queue.undo(id)).toBe(true);
    vi.advanceTimersByTime(10_000);

    expect(a.undo).toHaveBeenCalledTimes(1);
    expect(a.commit).not.toHaveBeenCalled();
  });

  it('undo after the commit is a no-op', () => {
    const queue = createPendingActionQueue({ windowMs: 6000 });
    const a = action();
    const id = queue.schedule(a);
    vi.advanceTimersByTime(6000);

    expect(queue.undo(id)).toBe(false);
    expect(a.undo).not.toHaveBeenCalled();
  });

  it('a new action commits the pending one at once', () => {
    const queue = createPendingActionQueue({ windowMs: 6000 });
    const first = action();
    const second = action();
    queue.schedule(first);
    vi.advanceTimersByTime(1000);
    queue.schedule(second);

    expect(first.commit).toHaveBeenCalledTimes(1);
    expect(second.commit).not.toHaveBeenCalled();
    vi.advanceTimersByTime(6000);
    expect(second.commit).toHaveBeenCalledTimes(1);
    expect(first.commit).toHaveBeenCalledTimes(1);
  });

  it('flushAll commits everything pending (app unload) exactly once', async () => {
    const queue = createPendingActionQueue({ windowMs: 6000 });
    const a = action();
    queue.schedule(a);

    await queue.flushAll();
    vi.advanceTimersByTime(6000);

    expect(a.commit).toHaveBeenCalledTimes(1);
    expect(queue.pendingCount()).toBe(0);
  });

  it('settled resolves when the action is committed or undone', async () => {
    const queue = createPendingActionQueue({ windowMs: 6000 });
    const a = action();
    const b = action();
    const idA = queue.schedule(a);
    const doneA = queue.settled(idA);
    queue.undo(idA);
    await expect(doneA).resolves.toBe('undone');

    const idB = queue.schedule(b);
    const doneB = queue.settled(idB);
    vi.advanceTimersByTime(6000);
    await expect(doneB).resolves.toBe('committed');
  });

  it('a failing commit is reported, not thrown into the timer', async () => {
    const onError = vi.fn();
    const queue = createPendingActionQueue({ windowMs: 10, onError });
    const err = new Error('boom');
    const id = queue.schedule({ commit: async () => Promise.reject(err), undo: vi.fn() });
    const done = queue.settled(id);

    vi.advanceTimersByTime(10);

    await expect(done).resolves.toBe('committed');
    expect(onError).toHaveBeenCalledWith(err);
  });
});
