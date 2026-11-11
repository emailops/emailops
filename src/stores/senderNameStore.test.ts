// The names inbox rows give the recipients of the user's own messages: one map
// per account, loaded once per session and again when the account finishes a
// sync. A late answer to an older request must never overwrite a newer one,
// and a failure costs names (rows show addresses), never a retry storm.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const getSenderNames = vi.fn<(accountId: string) => Promise<Record<string, string>>>();

vi.mock('@/lib/api', () => ({
  getSenderNames: (accountId: string) => getSenderNames(accountId),
}));

import { useLogStore } from './logStore';
import {
  finishedSyncing,
  INITIAL_SENDER_NAMES,
  reduceFailed,
  reduceLoaded,
  reduceRequested,
  useSenderNameStore,
} from './senderNameStore';

describe('sender-name reducers', () => {
  it('stores the map of the request in flight', () => {
    const state = reduceLoaded(reduceRequested(INITIAL_SENDER_NAMES, 'acc', 1), 'acc', 1, { 'a@example.com': 'Ana' });
    expect(state.namesByAccount).toEqual({ acc: { 'a@example.com': 'Ana' } });
    expect(state.requests).toEqual({});
  });

  it('drops the answer to a request a newer one replaced', () => {
    const inFlight = reduceRequested(reduceRequested(INITIAL_SENDER_NAMES, 'acc', 1), 'acc', 2);
    const stale = reduceLoaded(inFlight, 'acc', 1, { 'a@example.com': 'Old' });
    expect(stale).toBe(inFlight);
  });

  it('leaves an account that never loaded with an empty map, so it is not asked again', () => {
    const state = reduceFailed(reduceRequested(INITIAL_SENDER_NAMES, 'acc', 1), 'acc', 1);
    expect(state.namesByAccount).toEqual({ acc: {} });
    expect(state.requests).toEqual({});
  });

  it('keeps the names it had when a reload fails', () => {
    const loaded = reduceLoaded(reduceRequested(INITIAL_SENDER_NAMES, 'acc', 1), 'acc', 1, { 'a@example.com': 'Ana' });
    const state = reduceFailed(reduceRequested(loaded, 'acc', 2), 'acc', 2);
    expect(state.namesByAccount).toEqual({ acc: { 'a@example.com': 'Ana' } });
  });
});

describe('finishedSyncing', () => {
  it('names the accounts that left the syncing set', () => {
    expect(finishedSyncing(new Set(['a', 'b']), new Set(['b', 'c']))).toEqual(['a']);
  });

  it('is empty while nothing finished', () => {
    expect(finishedSyncing(new Set(['a']), new Set(['a', 'b']))).toEqual([]);
  });
});

describe('useSenderNameStore', () => {
  beforeEach(() => {
    getSenderNames.mockReset();
    useSenderNameStore.setState(INITIAL_SENDER_NAMES);
    useLogStore.setState({ entries: [] });
  });

  it('loads an account once per session', async () => {
    getSenderNames.mockResolvedValue({ 'a@example.com': 'Ana' });
    const { ensureLoaded } = useSenderNameStore.getState();
    await Promise.all([ensureLoaded('acc'), ensureLoaded('acc')]);
    await ensureLoaded('acc');
    expect(getSenderNames).toHaveBeenCalledTimes(1);
    expect(useSenderNameStore.getState().namesByAccount.acc).toEqual({ 'a@example.com': 'Ana' });
  });

  it('reloads only an account it already loaded', async () => {
    getSenderNames.mockResolvedValue({});
    await useSenderNameStore.getState().reload('acc');
    expect(getSenderNames).not.toHaveBeenCalled();

    await useSenderNameStore.getState().ensureLoaded('acc');
    getSenderNames.mockResolvedValue({ 'a@example.com': 'Ana Ruiz' });
    await useSenderNameStore.getState().reload('acc');
    expect(getSenderNames).toHaveBeenCalledTimes(2);
    expect(useSenderNameStore.getState().namesByAccount.acc).toEqual({ 'a@example.com': 'Ana Ruiz' });
  });

  it('logs one error per failed request', async () => {
    getSenderNames.mockRejectedValue(new Error('database is locked'));
    await useSenderNameStore.getState().ensureLoaded('acc');
    await useSenderNameStore.getState().ensureLoaded('acc');
    const errors = useLogStore.getState().entries.filter((l) => l.level === 'error');
    expect(errors).toHaveLength(1);
    expect(errors[0].source).toBe('system');
  });
});
