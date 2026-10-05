// Thread actions (read/unread, star, archive, move to inbox): the pure
// reducers behind the optimistic update and its rollback, and the store
// actions that call them around the backend.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  applyThreadAction: vi.fn(),
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  getThread: vi.fn(),
  getEmailBody: vi.fn(async () => 'body'),
  markAsRead: vi.fn(async () => undefined),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import {
  applyThreadFlag,
  isThreadStarred,
  leavesList,
  pendingThreadActions,
  removeThreads,
  restoreThreads,
  threadKey,
  UNDO_WINDOW_MS,
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

  it('an undo brings the conversation back to the list without replacing the one now open', () => {
    const a = email('a', 't1');
    const b = email('b', 't2');
    const before = slices([a, b], { selectedEmail: a, threadEmails: [a] });
    const advanced = { ...removeThreads(before, keysOf('t1')), selectedEmail: b, threadEmails: [b] };

    const restored = restoreThreads(advanced, before, keysOf('t1'));

    expect(restored.emails.map((e) => e.id)).toEqual(['a', 'b']);
    expect(restored.selectedEmail?.id).toBe('b');
    expect(restored.threadEmails).toEqual([b]);
  });

  it('an undo reopens the conversation when nothing else was opened', () => {
    const a = email('a', 't1');
    const before = slices([a], { selectedEmail: a, threadEmails: [a] });

    const restored = restoreThreads(removeThreads(before, keysOf('t1')), before, keysOf('t1'));

    expect(restored.selectedEmail?.id).toBe('a');
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
    ['delete', 'inbox', true],
    ['delete', 'starred', true],
    ['delete', 'search', true],
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

describe('archive and delete wait out the undo window', () => {
  const t1 = { accountId: 'acc', threadId: 't1' };
  const t2 = { accountId: 'acc', threadId: 't2' };

  beforeAll(async () => {
    await initI18n('en');
  });

  beforeEach(async () => {
    vi.useFakeTimers();
    await pendingThreadActions.flushAll();
    useEmailStore.getState().reset();
    useToastStore.setState({ toasts: [] });
    vi.clearAllMocks();
    vi.mocked(api.applyThreadAction).mockResolvedValue({ failed: [] });
  });
  afterEach(() => vi.useRealTimers());

  const ids = () => useEmailStore.getState().emails.map((e) => e.id);

  it('archive leaves the list at once but reaches the provider only when the window closes', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });

    const done = useEmailStore.getState().archiveThreads([t1]);
    expect(ids()).toEqual(['b']);
    expect(api.applyThreadAction).not.toHaveBeenCalled();
    const [toast] = useToastStore.getState().toasts;
    expect(toast.message).toBe('Archived 1 conversation');
    expect(toast.actionLabel).toBe('Undo');
    expect(toast.durationMs).toBe(UNDO_WINDOW_MS);

    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    await done;

    expect(api.applyThreadAction).toHaveBeenCalledWith([t1], 'archive');
    expect(ids()).toEqual(['b']);
    expect(useToastStore.getState().toasts).toEqual([]);
  });

  it('a delete right after a sticky toast is closed still offers Undo for the whole window', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    const sticky = useToastStore.getState().addToast({ message: 'Message could not be sent', sticky: true });
    useToastStore.getState().dismissToast(sticky);

    const done = useEmailStore.getState().deleteThreads([t1, t2]);
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS - 1);

    const toasts = useToastStore.getState().toasts;
    expect(toasts.map((t) => t.message)).toEqual(['Deleted 2 conversations']);
    expect(api.applyThreadAction).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    await done;
  });

  it('undo puts the conversations back and never calls the provider', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });

    const done = useEmailStore.getState().deleteThreads([t1, t2]);
    expect(ids()).toEqual([]);
    useToastStore.getState().toasts[0].onAction?.();
    await done;
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS * 2);

    expect(ids()).toEqual(['a', 'b']);
    expect(useEmailStore.getState().totalCount).toBe(2);
    expect(api.applyThreadAction).not.toHaveBeenCalled();
  });

  it('delete sends one call for every conversation', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'starred' });

    const done = useEmailStore.getState().deleteThreads([t1, t2]);
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    await done;

    expect(api.applyThreadAction).toHaveBeenCalledTimes(1);
    expect(api.applyThreadAction).toHaveBeenCalledWith([t1, t2], 'delete');
    expect(ids()).toEqual([]);
  });

  it('a second action commits the first at once', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });

    const first = useEmailStore.getState().archiveThreads([t1]);
    const second = useEmailStore.getState().deleteThreads([t2]);
    await first;

    expect(api.applyThreadAction).toHaveBeenCalledWith([t1], 'archive');
    expect(api.applyThreadAction).not.toHaveBeenCalledWith([t2], 'delete');
    expect(useToastStore.getState().toasts).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    await second;
    expect(api.applyThreadAction).toHaveBeenCalledWith([t2], 'delete');
  });

  it('a refetch inside the window does not bring the rows back', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    vi.mocked(api.getEmails).mockResolvedValue([email('a', 't1'), email('b', 't2')]);
    vi.mocked(api.getEmailCount).mockResolvedValue(2);

    const done = useEmailStore.getState().archiveThreads([t1]);
    await useEmailStore.getState().fetchEmails('acc', null, [], true, 'inbox');
    expect(ids()).toEqual(['b']);

    useToastStore.getState().toasts[0].onAction?.();
    await done;
    expect(ids()).toEqual(['a', 'b']);
  });

  it('opening another view commits what is pending first', async () => {
    useEmailStore.setState({ emails: [email('a', 't1')], totalCount: 1, listScope: 'inbox' });

    const done = useEmailStore.getState().archiveThreads([t1]);
    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'archive');
    await done;

    expect(api.applyThreadAction).toHaveBeenCalledWith([t1], 'archive');
    expect(vi.mocked(api.applyThreadAction).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(api.getEmails).mock.invocationCallOrder[0],
    );
  });

  it('a failed commit puts back exactly the refused conversations with one toast', async () => {
    useEmailStore.setState({ emails: [email('a', 't1'), email('b', 't2')], totalCount: 2, listScope: 'inbox' });
    vi.mocked(api.applyThreadAction).mockResolvedValue({
      failed: [{ accountId: 'acc', threadId: 't2', code: 'no_archive_folder', params: {}, message: 'no folder' }],
    });

    const done = useEmailStore.getState().archiveThreads([t1, t2]);
    await vi.advanceTimersByTimeAsync(UNDO_WINDOW_MS);
    await done;

    expect(ids()).toEqual(['b']);
    const toasts = useToastStore.getState().toasts;
    expect(toasts).toHaveLength(1);
    expect(toasts[0].actionLabel).toBeUndefined();
  });
});
