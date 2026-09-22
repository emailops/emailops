// navigateToEmail must hand the list back once it has loaded: a search or
// filter applied afterwards (e.g. the chat's "show emails in list") has to
// reach the backend instead of leaving the navigated list on screen.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Email } from '@/types';

const focused = { id: 'focused', accountId: 'acc', threadId: 't1', isRead: true, timestamp: 1 } as Email;
const cited = { id: 'cited', accountId: 'acc', threadId: 't2', isRead: true, timestamp: 2 } as Email;

vi.mock('@/lib/api', () => ({
  getEmailById: vi.fn(async () => focused),
  getEmailInboxPosition: vi.fn(async () => 0),
  getEmailCount: vi.fn(async () => 500),
  getEmails: vi.fn(async () => [focused]),
  getThread: vi.fn(async () => [focused]),
  getEmailBody: vi.fn(async () => ''),
  searchEmails: vi.fn(async () => ({ emails: [cited] })),
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

    expect(api.searchEmails).toHaveBeenCalledWith('acc', 'id:cited', true, ['primary']);
    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['cited']);
  });

  it('runs a later fetch when pre-seeded search results took over a navigation in flight', async () => {
    const store = useEmailStore.getState();
    const navigation = store.navigateToEmail('acc', 'focused');
    store.applySearchResults('id:cited', [cited]);
    await navigation;

    await useEmailStore.getState().fetchEmails('acc', null, ['primary'], false, undefined); // pre-seeded: no-op
    await useEmailStore.getState().fetchEmails('acc', null, ['primary'], false, undefined);

    expect(api.searchEmails).toHaveBeenCalledTimes(1);
  });

  it('keeps the category filter off the navigated list until the next fetch', async () => {
    await useEmailStore.getState().navigateToEmail('acc', 'focused');
    expect(useEmailStore.getState().navigationMode).toBe(true);
  });
});
