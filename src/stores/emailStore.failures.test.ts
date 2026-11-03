// A failure always shows: a first page takes the list's place, a later page
// leaves a Retry under the last row, and a background refresh only logs. After
// a failure the list never claims there is nothing to show.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ThreadPage } from '@/lib/api';
import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 1),
  listThreads: vi.fn(),
  listThreadSnoozes: vi.fn(async () => []),
  getJunkVerdicts: vi.fn(async () => ({})),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';
import { useJunkStore } from './junkStore';
import { useLogStore } from './logStore';

function row(id: string): Email {
  return { id, accountId: 'acc', threadId: `t-${id}`, isRead: false, timestamp: 1 } as Email;
}

const emptyPage: ThreadPage = { emails: [], threadLatestAt: {}, nextCursor: null, parsedQuery: null };
const levels = () => useLogStore.getState().entries.map((l) => l.level);

describe('list failures', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useJunkStore.setState({ verdictsByEmail: {}, loaded: {} });
    useLogStore.setState({ entries: [] });
    useEmailStore.getState().reset();
    vi.mocked(api.getEmails).mockResolvedValue([]);
    vi.mocked(api.listThreads).mockResolvedValue(emptyPage);
  });

  it('a failed first page takes the place of the list, even over another view', async () => {
    useEmailStore.setState({ emails: [row('from-another-view')] });
    vi.mocked(api.getEmails).mockRejectedValue(new Error('offline'));

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'sent');

    const s = useEmailStore.getState();
    expect(s.emails).toEqual([]);
    expect(s.listError).toContain('offline');
    expect(s.error).toBeNull();
    expect(s.paging.status).toBe('failed');
    expect(s.isLoading).toBe(false);
  });

  it('a new list forgets the failure of the last one', async () => {
    vi.mocked(api.getEmails).mockRejectedValueOnce(new Error('offline'));
    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    expect(useEmailStore.getState().listError).toBeNull();
  });

  it('a later page keeps the rows and stops asking until Retry', async () => {
    useEmailStore.setState({ emails: [row('a')], paging: { status: 'idle', cursor: null, offset: 1 }, totalCount: 9 });
    vi.mocked(api.getEmails).mockRejectedValueOnce(new Error('offline'));

    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');
    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    const s = useEmailStore.getState();
    expect(api.getEmails).toHaveBeenCalledTimes(1);
    expect(s.emails.map((e) => e.id)).toEqual(['a']);
    expect(s.paging.status).toBe('failed');
    expect(s.error).toBeNull();
    expect(s.listError).toBeNull();
  });

  it('a background refresh of a mailbox fails into the log and leaves the list', async () => {
    useEmailStore.setState({ emails: [row('a')] });
    vi.mocked(api.getEmails).mockRejectedValue(new Error('offline'));

    await useEmailStore.getState().fetchEmails('acc', null, [], true, 'inbox');

    const s = useEmailStore.getState();
    expect(s.emails.map((e) => e.id)).toEqual(['a']);
    expect(s.error).toBeNull();
    expect(s.listError).toBeNull();
    expect(levels()).toContain('error');
  });

  it('a late failure of the previous list does not mark the new one', async () => {
    useEmailStore.setState({ emails: [row('a')], paging: { status: 'idle', cursor: null, offset: 1 }, totalCount: 9 });
    let fail: (reason: Error) => void = () => {};
    vi.mocked(api.getEmails).mockReturnValueOnce(
      new Promise((_, reject) => {
        fail = reject;
      }),
    );
    const paging = useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    useEmailStore.setState({
      currentFetchId: useEmailStore.getState().currentFetchId + 1,
      paging: { status: 'idle', cursor: null, offset: 5 },
    });
    fail(new Error('offline'));
    await paging;

    expect(useEmailStore.getState().paging.status).toBe('idle');
  });

  it('Retry repeats the first page when there are no rows', async () => {
    vi.mocked(api.getEmails).mockRejectedValueOnce(new Error('offline'));
    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');
    vi.mocked(api.getEmails).mockResolvedValue([row('a')]);

    await useEmailStore.getState().retryList('acc', 'inbox');

    const s = useEmailStore.getState();
    expect(s.emails.map((e) => e.id)).toEqual(['a']);
    expect(s.listError).toBeNull();
  });

  it('Retry repeats the next page when there are rows', async () => {
    useEmailStore.setState({
      emails: [row('a')],
      paging: { status: 'failed', cursor: null, offset: 1 },
      totalCount: 2,
    });
    vi.mocked(api.getEmails).mockResolvedValue([row('b')]);

    await useEmailStore.getState().retryList('acc', 'inbox');

    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['a', 'b']);
    expect(api.getEmails).toHaveBeenCalledWith('acc', 50, 1, 'inbox', false);
  });
});

describe('list logs', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useLogStore.setState({ entries: [] });
    useEmailStore.getState().reset();
    vi.mocked(api.listThreads).mockResolvedValue(emptyPage);
  });

  it('reports a search the user asked for, without its text', async () => {
    useEmailStore.setState({ searchQuery: 'from:ana@empresa.example' });

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    const logs = useLogStore.getState().entries;
    expect(logs.map((l) => l.level)).toEqual(['info', 'success']);
    expect(logs.map((l) => l.message).join(' ')).not.toContain('ana');
  });

  it('stays quiet through a refresh', async () => {
    useEmailStore.setState({ searchQuery: 'invoice', emails: [row('a')] });

    await useEmailStore.getState().fetchEmails('acc', null, [], true, 'inbox');

    expect(levels()).toEqual([]);
  });
});
