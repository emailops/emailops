// The list's state must only change for what happened to the list: a search
// applied twice, an undo flush, a background refresh or a failed page must not
// leave it showing the wrong mail or stuck.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  getJunkVerdicts: vi.fn(async () => ({})),
  getEmails: vi.fn(),
  getEmailCount: vi.fn(async () => 500),
  listThreads: vi.fn(async () => ({ emails: [], threadLatestAt: {}, nextCursor: null, parsedQuery: null })),
  listThreadSnoozes: vi.fn(async () => []),
  applyThreadAction: vi.fn(),
  getThread: vi.fn(async () => []),
  getEmailBody: vi.fn(async () => ''),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';

function row(id: string, accountId = 'acc', threadId = `thread-${id}`): Email {
  return { id, accountId, threadId, mailbox: 'inbox', isRead: true, timestamp: 1, subject: id } as Email;
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

beforeEach(() => {
  useEmailStore.getState().reset();
  vi.clearAllMocks();
  vi.mocked(api.getEmails).mockResolvedValue([row('listed')]);
});

describe('a list opened while an undo window is flushing', () => {
  it('does not replace a list opened after it', async () => {
    const commit = deferred<{ failed: [] }>();
    vi.mocked(api.applyThreadAction).mockReturnValue(commit.promise as never);
    vi.mocked(api.getEmails).mockImplementation(async (_account, _limit, _offset, mailbox) => [row(String(mailbox))]);

    void useEmailStore.getState().archiveThreads([{ accountId: 'acc', threadId: 'thread-old' }]);
    const archive = useEmailStore.getState().fetchEmails('acc', null, [], false, 'archive');
    // The queue is already drained: this one does not wait for the commit.
    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'sent');

    commit.resolve({ failed: [] });
    await archive;

    const s = useEmailStore.getState();
    expect(s.emails.map((e) => e.id)).toEqual(['sent']);
    expect(s.listScope).toBe('sent');
    expect(s.isLoading).toBe(false);
  });

  it('shows the loading state while it waits for the flush', async () => {
    const commit = deferred<{ failed: [] }>();
    vi.mocked(api.applyThreadAction).mockReturnValue(commit.promise as never);

    void useEmailStore.getState().archiveThreads([{ accountId: 'acc', threadId: 'thread-old' }]);
    const archive = useEmailStore.getState().fetchEmails('acc', null, [], false, 'archive');

    expect(useEmailStore.getState().isLoading).toBe(true);
    commit.resolve({ failed: [] });
    await archive;
    expect(useEmailStore.getState().isLoading).toBe(false);
  });
});

describe('a background refresh', () => {
  it('leaves the focused email, the navigated list and the error on screen alone', async () => {
    useEmailStore.setState({ error: 'Could not load the thread', focusEmailId: 'e9', navigationMode: true });

    await useEmailStore.getState().fetchEmails('acc', null, [], true, 'inbox');

    const s = useEmailStore.getState();
    expect(s.error).toBe('Could not load the thread');
    expect(s.focusEmailId).toBe('e9');
    expect(s.navigationMode).toBe(true);
    expect(s.emails.map((e) => e.id)).toEqual(['listed']);
  });

  it('an explicit load clears them', async () => {
    useEmailStore.setState({ error: 'Could not load the thread', focusEmailId: 'e9', navigationMode: true });

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    const s = useEmailStore.getState();
    expect(s.error).toBeNull();
    expect(s.focusEmailId).toBeNull();
    expect(s.navigationMode).toBe(false);
  });
});

describe('load more after a failed page', () => {
  it('stays off for scrolling, and comes back when the user asks again', async () => {
    useEmailStore.setState({
      emails: [row('first')],
      paging: { status: 'idle', cursor: null, offset: 1 },
      totalCount: 500,
    });
    vi.mocked(api.getEmails).mockRejectedValueOnce(new Error('offline'));

    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');
    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');
    expect(api.getEmails).toHaveBeenCalledTimes(1);

    useEmailStore.getState().retryLoadMore();
    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    expect(api.getEmails).toHaveBeenCalledTimes(2);
    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['first', 'listed']);
  });
});

describe('thread tabs', () => {
  it('opens one tab per account when two accounts share a thread id', async () => {
    const work = row('e1', 'work', 'same-thread');
    const home = row('e2', 'home', 'same-thread');

    await useEmailStore.getState().openTab(work);
    await useEmailStore.getState().openTab(home);

    const s = useEmailStore.getState();
    expect(s.tabs.map((t) => t.accountId)).toEqual(['work', 'home']);
    expect(s.activeTabId).toBe(s.tabs[1].id);
  });

  it('reopening a thread goes back to its tab', async () => {
    const work = row('e1', 'work', 'same-thread');

    await useEmailStore.getState().openTab(work);
    await useEmailStore.getState().openTab(work);

    expect(useEmailStore.getState().tabs).toHaveLength(1);
  });
});
