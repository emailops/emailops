// navigateToEmail must hand the list back once it has loaded: a search or
// filter applied afterwards (e.g. the chat's "show emails in list") has to
// reach the backend instead of leaving the navigated list on screen.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

const focused = { id: 'focused', accountId: 'acc', threadId: 't1', isRead: true, timestamp: 1 } as Email;
const cited = { id: 'cited', accountId: 'acc', threadId: 't2', isRead: true, timestamp: 2 } as Email;

vi.mock('@/lib/api', () => ({
  getJunkVerdicts: vi.fn(async () => ({})),
  getEmailById: vi.fn(async () => focused),
  getEmailInboxPosition: vi.fn(async () => 0),
  getEmailCount: vi.fn(async () => 500),
  getEmails: vi.fn(async () => [focused]),
  getThread: vi.fn(async () => [focused]),
  getEmailBody: vi.fn(async () => ''),
  listThreads: vi.fn(async () => ({ emails: [cited], threadLatestAt: {}, nextCursor: null, parsedQuery: null })),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';

describe('fetchEmails after navigateToEmail', () => {
  beforeEach(() => {
    useEmailStore.getState().reset();
    vi.clearAllMocks();
  });

  it('runs a search applied once the navigation has finished', async () => {
    const store = useEmailStore.getState();
    await store.navigateToEmail('acc', 'focused');

    store.setSearchQuery('id:cited');
    await useEmailStore.getState().fetchEmails('acc', null, ['primary'], false, undefined);

    // Search spans every category: the selected tab is not sent.
    expect(vi.mocked(api.listThreads).mock.calls[0]).toEqual(['acc', 'id:cited', 'inbox', false, null, 50, true]);
    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['cited']);
  });

  it('keeps the category filter off the navigated list until the next fetch', async () => {
    await useEmailStore.getState().navigateToEmail('acc', 'focused');
    expect(useEmailStore.getState().navigationMode).toBe(true);
  });
});
