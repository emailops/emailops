// Thread actions (read/unread, star, archive, move to inbox): the pure
// reducers behind the optimistic update and its rollback, and the store
// actions that call them around the backend.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  applyThreadAction: vi.fn(),
  getThread: vi.fn(),
  getEmailBody: vi.fn(async () => 'body'),
  markAsRead: vi.fn(async () => undefined),
}));

import * as api from '@/lib/api';
import {
  applyThreadFlag,
  isThreadStarred,
  leavesList,
  removeThreads,
  restoreThreads,
  threadKey,
  useEmailStore,
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

const slices = (emails: Email[], extra: Partial<ReturnType<typeof useEmailStore.getState>> = {}) => ({
  emails,
  threadEmails: [] as Email[],
  selectedEmail: null as Email | null,
  totalCount: emails.length,
  tabs: [],
  ...extra,
});

const keysOf = (...threads: string[]) => new Set(threads.map((t) => threadKey('acc', t)));

describe('applyThreadFlag', () => {
  it('marks the list rows of the threads and leaves other threads alone', () => {
    const state = slices([email('a', 't1'), email('b', 't2')]);
    const next = applyThreadFlag(state, keysOf('t1'), 'isRead', false);
    expect(next.emails.map((e) => e.isRead)).toEqual([false, true]);
  });

  it('stars the latest message of an open thread and unstars all of them', () => {
    const thread = [email('a1', 't1', { timestamp: 1 }), email('a2', 't1', { timestamp: 2 })];
    const state = slices([thread[1]], { threadEmails: thread, selectedEmail: thread[1] });

    const starred = applyThreadFlag(state, keysOf('t1'), 'isStarred', true);
    expect(starred.threadEmails.map((e) => e.isStarred)).toEqual([false, true]);
    expect(starred.selectedEmail?.isStarred).toBe(true);
    expect(isThreadStarred(starred.threadEmails)).toBe(true);

    const unstarred = applyThreadFlag(
      { ...starred, threadEmails: starred.threadEmails.map((e) => ({ ...e, isStarred: true })) },
      keysOf('t1'),
      'isStarred',
      false,
    );
    expect(unstarred.threadEmails.every((e) => !e.isStarred)).toBe(true);
  });

  it('reads every message of an open thread', () => {
    const thread = [email('a1', 't1', { isRead: false }), email('a2', 't1', { isRead: false })];
    const next = applyThreadFlag(slices([], { threadEmails: thread }), keysOf('t1'), 'isRead', true);
    expect(next.threadEmails.every((e) => e.isRead)).toBe(true);
  });
});

describe('removeThreads / restoreThreads', () => {
  it('drops the threads from the list, the count and the selection', () => {
    const a = email('a', 't1');
    const state = slices([a, email('b', 't2')], { selectedEmail: a, threadEmails: [a], totalCount: 10 });

    const next = removeThreads(state, keysOf('t1'));

    expect(next.emails.map((e) => e.id)).toEqual(['b']);
    expect(next.totalCount).toBe(9);
    expect(next.selectedEmail).toBeNull();
    expect(next.threadEmails).toEqual([]);
  });

  it('puts back only the failed threads, in their old place, keeping later changes to the rest', () => {
    const before = slices([email('a', 't1'), email('b', 't2'), email('c', 't3')]);
    const removed = removeThreads(before, keysOf('t1', 't2'));
    const later = { ...removed, emails: removed.emails.map((e) => ({ ...e, subject: 'changed' })) };

    const restored = restoreThreads(later, before, keysOf('t1'));

    expect(restored.emails.map((e) => [e.id, e.subject])).toEqual([
      ['a', 'a'],
      ['c', 'changed'],
    ]);
    expect(restored.totalCount).toBe(2);
  });

  it('restores the flags of failed threads', () => {
    const before = slices([email('a', 't1'), email('b', 't2')]);
    const patched = applyThreadFlag(before, keysOf('t1', 't2'), 'isStarred', true);

    const restored = restoreThreads(patched, before, keysOf('t2'));

    expect(restored.emails.map((e) => e.isStarred)).toEqual([true, false]);
  });
});

describe('leavesList', () => {
  it.each([
    ['archive', 'inbox', true],
    ['archive', 'starred', false],
    ['archive', 'search', false],
    ['moveToInbox', 'archive', true],
    ['moveToInbox', 'folder:Archive', true],
    ['moveToInbox', 'inbox', false],
    ['unstar', 'starred', true],
    ['unstar', 'inbox', false],
    ['markUnread', 'inbox', false],
    ['star', 'inbox', false],
  ] as const)('%s in the %s list → %s', (action, list, expected) => {
    expect(leavesList(action, list)).toBe(expected);
  });
});

describe('thread action store actions', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    useToastStore.setState({ toasts: [] });
    vi.clearAllMocks();
  });

  it('archives optimistically and keeps the result when the backend applies it', async () => {
    const a = email('a', 't1');
    useEmailStore.setState({ emails: [a, email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    vi.mocked(api.applyThreadAction).mockResolvedValue({ failed: [] });

    const pending = useEmailStore.getState().archiveThreads([{ accountId: 'acc', threadId: 't1' }]);
    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['b']);
    await pending;

    expect(api.applyThreadAction).toHaveBeenCalledWith([{ accountId: 'acc', threadId: 't1' }], 'archive');
    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['b']);
    expect(useToastStore.getState().toasts).toEqual([]);
  });

  it('rolls back the threads the backend could not change and says so', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    vi.mocked(api.applyThreadAction).mockResolvedValue({
      failed: [{ accountId: 'acc', threadId: 't2', code: 'no_archive_folder', params: {}, message: 'no folder' }],
    });

    await useEmailStore.getState().archiveThreads([
      { accountId: 'acc', threadId: 't1' },
      { accountId: 'acc', threadId: 't2' },
    ]);

    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['b']);
    expect(useToastStore.getState().toasts).toHaveLength(1);
  });

  it('rolls everything back when the call itself fails', async () => {
    useEmailStore.setState({ emails: [email('a', 't1')], totalCount: 1, listScope: 'inbox' });
    vi.mocked(api.applyThreadAction).mockRejectedValue(new Error('ipc down'));

    await useEmailStore.getState().setThreadsStarred([{ accountId: 'acc', threadId: 't1' }], true);

    expect(useEmailStore.getState().emails[0].isStarred).toBe(false);
    expect(useToastStore.getState().toasts).toHaveLength(1);
  });

  it('marking unread leaves the open thread so it is not read again at once', async () => {
    const a = email('a', 't1');
    useEmailStore.setState({ emails: [a], selectedEmail: a, threadEmails: [a], listScope: 'inbox' });
    vi.mocked(api.applyThreadAction).mockResolvedValue({ failed: [] });

    await useEmailStore.getState().setThreadsRead([{ accountId: 'acc', threadId: 't1' }], false);

    const state = useEmailStore.getState();
    expect(state.selectedEmail).toBeNull();
    expect(state.emails[0].isRead).toBe(false);
    expect(api.applyThreadAction).toHaveBeenCalledWith([{ accountId: 'acc', threadId: 't1' }], 'markUnread');
  });

  it('starring keeps the row in the list and the selection', async () => {
    const a = email('a', 't1');
    useEmailStore.setState({ emails: [a], selectedEmail: a, threadEmails: [a], listScope: 'inbox' });
    vi.mocked(api.applyThreadAction).mockResolvedValue({ failed: [] });

    await useEmailStore.getState().setThreadsStarred([{ accountId: 'acc', threadId: 't1' }], true);

    const state = useEmailStore.getState();
    expect(state.emails[0].isStarred).toBe(true);
    expect(state.selectedEmail?.id).toBe('a');
  });

  it('an empty selection does nothing', async () => {
    await useEmailStore.getState().archiveThreads([]);
    expect(api.applyThreadAction).not.toHaveBeenCalled();
  });
});
