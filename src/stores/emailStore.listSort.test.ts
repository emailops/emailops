// The full-width list's column headers sort the whole list: every page and
// every background refetch asks the backend for the same order.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

const focused = { id: 'focused', accountId: 'acc', threadId: 't1', isRead: true, timestamp: 1 } as Email;

vi.mock('@/lib/api', () => ({
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  listThreads: vi.fn(async () => ({ emails: [], threadLatestAt: {}, nextCursor: null, parsedQuery: null })),
  listThreadSnoozes: vi.fn(async () => []),
  getJunkVerdicts: vi.fn(async () => ({})),
  getEmailById: vi.fn(async () => focused),
  getEmailInboxPosition: vi.fn(async () => 0),
  getThread: vi.fn(async () => [focused]),
  getEmailBody: vi.fn(async () => ''),
}));

import * as api from '@/lib/api';
import { DEFAULT_LIST_SORT } from '@/lib/listColumns';
import { useEmailStore } from './emailStore';

const bySize = { key: 'size', dir: 'desc' } as const;

describe('the list sort', () => {
  beforeEach(() => {
    useEmailStore.setState({ listSort: DEFAULT_LIST_SORT, searchQuery: null, emails: [], totalCount: 0 });
    vi.clearAllMocks();
  });

  it('asks the mailbox list for the chosen order', async () => {
    useEmailStore.getState().setListSort(bySize);

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    expect(api.getEmails).toHaveBeenCalledWith('acc', 50, 0, 'inbox', false, bySize);
  });

  it('asks the next page for the same order', async () => {
    useEmailStore.getState().setListSort(bySize);
    useEmailStore.setState({
      emails: [{ id: 'a', accountId: 'acc', threadId: 't-a' } as Email],
      paging: { status: 'idle', cursor: null, offset: 1 },
      totalCount: 2,
    });

    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    expect(api.getEmails).toHaveBeenCalledWith('acc', 50, 1, 'inbox', false, bySize);
  });

  it('goes back to newest first when the app jumps to a message', async () => {
    useEmailStore.getState().setListSort(bySize);

    await useEmailStore.getState().navigateToEmail('acc', 'focused');

    // The jump places the message by its date in the newest-first list.
    expect(useEmailStore.getState().listSort).toEqual(DEFAULT_LIST_SORT);
  });
});
