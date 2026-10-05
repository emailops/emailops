// Snooze: the pure reducers over the snooze map and the store actions that
// snooze / unsnooze conversations around the backend.

import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  applyThreadAction: vi.fn(),
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  listThreadSnoozes: vi.fn(async () => []),
  snoozeThreads: vi.fn(),
  unsnoozeThreads: vi.fn(),
}));

import { initI18n } from '@/i18n';
import type { ThreadSnooze } from '@/lib/api';
import * as api from '@/lib/api';
import {
  isBackFromSnooze,
  isSnoozed,
  snoozeLeavesList,
  snoozeMap,
  threadKey,
  UNDO_WINDOW_MS,
  useEmailStore,
  withoutSnoozes,
  withSnoozes,
} from './emailStore';
import { useToastStore } from './toastStore';

function email(id: string, threadId: string, extra: Partial<Email> = {}): Email {
  return {
    id,
    accountId: 'acc',
    threadId,
    isRead: true,
    isStarred: false,
    timestamp: 1,
    subject: id,
    mailbox: 'inbox',
    isSent: false,
    ...extra,
  } as Email;
}

const record = (threadId: string, extra: Partial<ThreadSnooze> = {}): ThreadSnooze => ({
  accountId: 'acc',
  threadId,
  snoozedUntil: 2_000,
  createdAt: 1_000,
  wokeAt: null,
  ...extra,
});

const t1 = { accountId: 'acc', threadId: 't1' };
const t2 = { accountId: 'acc', threadId: 't2' };

describe('snooze map reducers', () => {
  it('keys records by conversation and answers snoozed / back-from-snooze', () => {
    const map = snoozeMap([record('t1'), record('t2', { wokeAt: 1_500 })]);
    expect(isSnoozed(map, email('a', 't1'))).toBe(true);
    expect(isSnoozed(map, email('b', 't2'))).toBe(false);
    // Woken: marked until the conversation is read.
    expect(isBackFromSnooze(map, email('b', 't2', { isRead: false }))).toBe(true);
    expect(isBackFromSnooze(map, email('b', 't2', { isRead: true }))).toBe(false);
    expect(isBackFromSnooze(map, email('a', 't1', { isRead: false }))).toBe(false);
    // Another account's same thread id is a different conversation.
    expect(isSnoozed(map, email('c', 't1', { accountId: 'other' }))).toBe(false);
  });

  it('adds and removes snoozes without touching the others', () => {
    const map = snoozeMap([record('t2', { wokeAt: 1_500 })]);
    const added = withSnoozes(map, [t1, t2], 9_000, 100);
    expect(added.get(threadKey('acc', 't1'))).toEqual(record('t1', { snoozedUntil: 9_000, createdAt: 100 }));
    expect(added.get(threadKey('acc', 't2'))?.wokeAt).toBeNull();
    const removed = withoutSnoozes(added, new Set([threadKey('acc', 't1')]));
    expect([...removed.keys()]).toEqual([threadKey('acc', 't2')]);
    expect(map.get(threadKey('acc', 't2'))?.wokeAt).toBe(1_500);
  });

  it('snoozing leaves the inbox, unsnoozing leaves the Snoozed view', () => {
    expect(snoozeLeavesList('snooze', 'inbox')).toBe(true);
    expect(snoozeLeavesList('snooze', 'starred')).toBe(false);
    expect(snoozeLeavesList('snooze', 'search')).toBe(false);
    expect(snoozeLeavesList('unsnooze', 'snoozed')).toBe(true);
    expect(snoozeLeavesList('unsnooze', 'inbox')).toBe(false);
  });
});

describe('snooze store actions', () => {
  beforeAll(async () => {
    await initI18n('en');
  });

  beforeEach(() => {
    useEmailStore.getState().reset();
    useToastStore.setState({ toasts: [] });
    vi.clearAllMocks();
    vi.mocked(api.snoozeThreads).mockResolvedValue({ failed: [] });
    vi.mocked(api.unsnoozeThreads).mockResolvedValue(undefined);
  });

  const ids = () => useEmailStore.getState().emails.map((e) => e.id);

  it('snoozing takes the conversations out of the inbox and offers Undo', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });

    await useEmailStore.getState().snoozeThreads([t1], 1_900_000_000);

    expect(api.snoozeThreads).toHaveBeenCalledWith([t1], 1_900_000_000);
    expect(ids()).toEqual(['b']);
    expect(useEmailStore.getState().totalCount).toBe(1);
    expect(isSnoozed(useEmailStore.getState().snoozes, email('a', 't1'))).toBe(true);
    const [toast] = useToastStore.getState().toasts;
    expect(toast.message).toMatch(/^Snoozed until /);
    expect(toast.actionLabel).toBe('Undo');
    expect(toast.durationMs).toBe(UNDO_WINDOW_MS);
  });

  it('undo unsnoozes and puts the rows back', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    await useEmailStore.getState().snoozeThreads([t1, t2], 1_900_000_000);
    expect(ids()).toEqual([]);

    await useToastStore.getState().toasts[0].onAction?.();
    await vi.waitFor(() => expect(ids()).toEqual(['a', 'b']));

    expect(api.unsnoozeThreads).toHaveBeenCalledWith([t1, t2]);
    expect(useEmailStore.getState().snoozes.size).toBe(0);
  });

  it('a refused snooze comes back with one error toast', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    vi.mocked(api.snoozeThreads).mockResolvedValue({
      failed: [{ accountId: 'acc', threadId: 't2', code: 'not_found', params: {}, message: 'gone' }],
    });

    await useEmailStore.getState().snoozeThreads([t1, t2], 1_900_000_000);

    expect(ids()).toEqual(['b']);
    expect(isSnoozed(useEmailStore.getState().snoozes, email('b', 't2'))).toBe(false);
    const toasts = useToastStore.getState().toasts;
    expect(toasts.map((t) => t.actionLabel ?? null)).toEqual(['Undo', null]);
  });

  it('a failed call rolls everything back', async () => {
    useEmailStore.setState({ emails: [email('a', 't1')], totalCount: 1, listScope: 'inbox' });
    vi.mocked(api.snoozeThreads).mockRejectedValue(new Error('boom'));

    await useEmailStore.getState().snoozeThreads([t1], 1_900_000_000);

    expect(ids()).toEqual(['a']);
    expect(useEmailStore.getState().snoozes.size).toBe(0);
    expect(useToastStore.getState().toasts).toHaveLength(1);
  });

  it('snoozing from another view keeps the row', async () => {
    useEmailStore.setState({ emails: [email('a', 't1')], totalCount: 1, listScope: 'starred' });
    await useEmailStore.getState().snoozeThreads([t1], 1_900_000_000);
    expect(ids()).toEqual(['a']);
  });

  it('unsnoozing takes the conversation out of the Snoozed view', async () => {
    useEmailStore.setState({
      emails: [email('a', 't1'), email('b', 't2')],
      totalCount: 2,
      listScope: 'snoozed',
      snoozes: snoozeMap([record('t1'), record('t2')]),
    });

    await useEmailStore.getState().unsnoozeThreads([t1]);

    expect(api.unsnoozeThreads).toHaveBeenCalledWith([t1]);
    expect(ids()).toEqual(['b']);
    expect([...useEmailStore.getState().snoozes.keys()]).toEqual([threadKey('acc', 't2')]);
  });

  it('a failed unsnooze puts the row and the record back', async () => {
    useEmailStore.setState({
      emails: [email('a', 't1')],
      totalCount: 1,
      listScope: 'snoozed',
      snoozes: snoozeMap([record('t1')]),
    });
    vi.mocked(api.unsnoozeThreads).mockRejectedValue(new Error('boom'));

    await useEmailStore.getState().unsnoozeThreads([t1]);

    expect(ids()).toEqual(['a']);
    expect(useEmailStore.getState().snoozes.size).toBe(1);
    expect(useToastStore.getState().toasts).toHaveLength(1);
  });

  it('fetchSnoozes loads the records of the scope', async () => {
    vi.mocked(api.listThreadSnoozes).mockResolvedValue([record('t1')]);
    await useEmailStore.getState().fetchSnoozes(null);
    expect(api.listThreadSnoozes).toHaveBeenCalledWith(null);
    expect(isSnoozed(useEmailStore.getState().snoozes, email('a', 't1'))).toBe(true);
  });
});
