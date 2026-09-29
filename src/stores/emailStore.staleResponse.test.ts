// Async store actions must not write a response that no longer matches what is
// on screen: a thread fetched for a previous selection, or anything that lands
// after the store was reset for another account.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  getThread: vi.fn(),
  getEmailBody: vi.fn(async () => 'body'),
  getEmails: vi.fn(),
  getEmailCount: vi.fn(async () => 500),
  getEmailById: vi.fn(),
  getEmailInboxPosition: vi.fn(async () => 0),
  markAsRead: vi.fn(async () => undefined),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';

function email(id: string, threadId: string): Email {
  return { id, accountId: 'acc', threadId, isRead: true, timestamp: 1, subject: id } as Email;
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const a = email('a', 'thread-a');
const b = email('b', 'thread-b');

describe('selectEmail with an out-of-order thread response', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    vi.clearAllMocks();
  });

  it('keeps the thread of the email selected last', async () => {
    const slowA = deferred<Email[]>();
    vi.mocked(api.getThread).mockImplementation((_acc, threadId) =>
      threadId === 'thread-a' ? slowA.promise : Promise.resolve([b]),
    );

    const first = useEmailStore.getState().selectEmail(a);
    await useEmailStore.getState().selectEmail(b);
    slowA.resolve([a]);
    await first;

    const s = useEmailStore.getState();
    expect(s.selectedEmail?.id).toBe('b');
    expect(s.threadEmails.map((e) => e.id)).toEqual(['b']);
    expect(s.isLoadingThread).toBe(false);
  });

  it('ignores a failed thread load for an email no longer selected', async () => {
    const slowA = deferred<Email[]>();
    vi.mocked(api.getThread).mockImplementation((_acc, threadId) =>
      threadId === 'thread-a' ? slowA.promise : Promise.resolve([b]),
    );

    const first = useEmailStore.getState().selectEmail(a);
    await useEmailStore.getState().selectEmail(b);
    slowA.reject(new Error('boom'));
    await first;

    const s = useEmailStore.getState();
    expect(s.threadEmails.map((e) => e.id)).toEqual(['b']);
    expect(s.error).toBeNull();
  });

  it('does not write a thread that arrives after the selection was cleared', async () => {
    const slowA = deferred<Email[]>();
    vi.mocked(api.getThread).mockReturnValue(slowA.promise);

    const first = useEmailStore.getState().selectEmail(a);
    await useEmailStore.getState().selectEmail(null);
    slowA.resolve([a]);
    await first;

    expect(useEmailStore.getState().threadEmails).toEqual([]);
  });

  it('does not write a thread that arrives after a reset', async () => {
    const slowA = deferred<Email[]>();
    vi.mocked(api.getThread).mockReturnValue(slowA.promise);

    const first = useEmailStore.getState().selectEmail(a);
    useEmailStore.getState().reset();
    slowA.resolve([a]);
    await first;

    expect(useEmailStore.getState().threadEmails).toEqual([]);
    expect(useEmailStore.getState().selectedEmail).toBeNull();
  });
});

describe('navigateToEmail thread phase', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    vi.clearAllMocks();
  });

  it('does not overwrite a selection made while its thread was loading', async () => {
    const slowA = deferred<Email[]>();
    vi.mocked(api.getEmailById).mockResolvedValue(a);
    vi.mocked(api.getEmails).mockResolvedValue([a, b]);
    vi.mocked(api.getThread).mockImplementation((_acc, threadId) =>
      threadId === 'thread-a' ? slowA.promise : Promise.resolve([b]),
    );

    const nav = useEmailStore.getState().navigateToEmail('acc', 'a');
    // Let the list phase finish so the thread fetch for `a` is in flight.
    await vi.waitFor(() => expect(api.getThread).toHaveBeenCalled());
    await useEmailStore.getState().selectEmail(b);
    slowA.resolve([a]);
    await nav;

    expect(useEmailStore.getState().threadEmails.map((e) => e.id)).toEqual(['b']);
  });
});

describe('openTab with an out-of-order thread response', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    vi.clearAllMocks();
  });

  it('keeps the thread of a tab reopened while the first load was pending', async () => {
    const slow = deferred<Email[]>();
    const a2 = { ...email('a2', 'thread-a'), timestamp: 2 };
    vi.mocked(api.getThread).mockReturnValueOnce(slow.promise).mockResolvedValueOnce([a, a2]);

    const first = useEmailStore.getState().openTab(a);
    useEmailStore.getState().closeTab('thread-a');
    await useEmailStore.getState().openTab(a);
    slow.reject(new Error('boom'));
    await first;

    const tab = useEmailStore.getState().tabs[0];
    expect(tab.type === 'thread' && tab.threadEmails.map((e) => e.id)).toEqual(['a', 'a2']);
  });
});

describe('loadMoreEmails after a failure', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    vi.clearAllMocks();
    useEmailStore.setState({ emails: [a], hasMore: true, totalCount: 500 });
  });

  it('does not retry on its own until the list is fetched again', async () => {
    vi.mocked(api.getEmails).mockRejectedValue(new Error('offline'));

    await useEmailStore.getState().loadMoreEmails('acc', null, [], undefined);
    await useEmailStore.getState().loadMoreEmails('acc', null, [], undefined);

    expect(api.getEmails).toHaveBeenCalledTimes(1);
    expect(useEmailStore.getState().error).not.toBeNull();
  });

  it('pages again once the list has been refetched', async () => {
    vi.mocked(api.getEmails).mockRejectedValueOnce(new Error('offline'));
    await useEmailStore.getState().loadMoreEmails('acc', null, [], undefined);

    vi.mocked(api.getEmails).mockResolvedValue(Array.from({ length: 50 }, (_, i) => email(`e${i}`, `t${i}`)));
    await useEmailStore.getState().fetchEmails('acc', null, [], true, undefined);
    await useEmailStore.getState().loadMoreEmails('acc', null, [], undefined);

    expect(api.getEmails).toHaveBeenCalledTimes(3);
  });
});

describe('selectEmail without marking read', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    vi.clearAllMocks();
    vi.mocked(api.getThread).mockResolvedValue([]);
  });

  it('leaves an unread email unread when asked to', async () => {
    await useEmailStore.getState().selectEmail({ ...a, isRead: false }, undefined, { markRead: false });
    expect(api.markAsRead).not.toHaveBeenCalled();
  });

  it('marks an unread email read by default', async () => {
    await useEmailStore.getState().selectEmail({ ...a, isRead: false });
    expect(api.markAsRead).toHaveBeenCalledWith('a');
  });
});
