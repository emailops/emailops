// The list's Unread toggle reaches the backend on every list path, so the
// count, the pages and the background refetches all describe the same set.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  listThreads: vi.fn(async () => ({ emails: [], threadLatestAt: {}, nextCursor: null, parsedQuery: null })),
  listThreadSnoozes: vi.fn(async () => []),
  applyThreadAction: vi.fn(async () => ({ failed: [] })),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';

function unreadRow(id: string): Email {
  return { id, accountId: 'acc', threadId: `t-${id}`, isRead: false, timestamp: 1, subject: id } as Email;
}

describe('the Unread toggle', () => {
  beforeEach(() => {
    useEmailStore.setState({ unreadOnly: false, searchQuery: null, emails: [], totalCount: 0 });
    vi.clearAllMocks();
  });

  it('asks the mailbox list and its count for unread rows only', async () => {
    useEmailStore.getState().setUnreadOnly(true);

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'sent');

    expect(api.getEmails).toHaveBeenCalledWith('acc', 50, 0, 'sent', true);
    expect(api.getEmailCount).toHaveBeenCalledWith('acc', 'sent', true);
  });

  it('asks the next page for unread rows only', async () => {
    useEmailStore.getState().setUnreadOnly(true);
    useEmailStore.setState({
      emails: [unreadRow('a')],
      paging: { status: 'idle', cursor: null, offset: 1 },
      totalCount: 2,
    });

    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    expect(api.getEmails).toHaveBeenCalledWith('acc', 50, 1, 'inbox', true);
  });

  it('narrows a sidebar filter to unread conversations', async () => {
    useEmailStore.getState().setUnreadOnly(true);
    useEmailStore.setState({ searchQuery: 'from:bob@acme.com' });

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    expect(vi.mocked(api.listThreads).mock.calls[0]?.[3]).toBe(true);
  });

  it('survives a reset for another account', () => {
    useEmailStore.getState().setUnreadOnly(true);

    useEmailStore.getState().reset();

    expect(useEmailStore.getState().unreadOnly).toBe(true);
  });

  it('keeps a conversation marked read on screen until the list reloads', async () => {
    useEmailStore.getState().setUnreadOnly(true);
    useEmailStore.setState({ emails: [unreadRow('a')], listScope: 'inbox' });

    await useEmailStore.getState().setThreadsRead([{ accountId: 'acc', threadId: 't-a' }], true);

    const rows = useEmailStore.getState().emails;
    expect(rows.map((e) => e.id)).toEqual(['a']);
    expect(rows[0]?.isRead).toBe(true);
  });
});
