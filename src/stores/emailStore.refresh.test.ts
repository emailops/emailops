// A background refresh of a list with a query merges its first rows into the
// list: it never cuts a long list, drops a page in flight, or lands on another
// list. Plain mailboxes refresh as they always did.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ThreadPage } from '@/lib/api';
import type { Email, ThreadCursor } from '@/types';

vi.mock('@/lib/api', () => ({
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  listThreads: vi.fn(),
  listThreadSnoozes: vi.fn(async () => []),
  getJunkVerdicts: vi.fn(async () => ({})),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';
import { useJunkStore } from './junkStore';

/** Rows numbered from the top: row 0 sorts first. */
function row(n: number): Email {
  const ts = 100_000 - n;
  return { id: `e${n}`, accountId: 'acc', threadId: `t${n}`, isRead: false, timestamp: ts } as Email;
}

function rows(from: number, to: number): Email[] {
  return Array.from({ length: to - from }, (_, i) => row(from + i));
}

const cursorAt = (n: number): ThreadCursor => ({ threadTs: 100_000 - n, timestamp: 100_000 - n, id: `e${n}` });

function page(emails: Email[], nextCursor: ThreadCursor | null): ThreadPage {
  return {
    emails,
    threadLatestAt: Object.fromEntries(emails.map((e) => [e.id, e.timestamp])),
    nextCursor,
    parsedQuery: null,
  };
}

function loaded(count: number) {
  const emails = rows(0, count).map((e) => ({ ...e, threadLatestAt: e.timestamp }));
  useEmailStore.setState({
    searchQuery: 'invoice',
    emails,
    isLoading: false,
    error: null,
    paging: { status: 'idle', cursor: cursorAt(count - 1), offset: 0 },
  });
}

const refresh = () => useEmailStore.getState().fetchEmails('acc', null, [], true, 'inbox');
const ids = () => useEmailStore.getState().emails.map((e) => e.id);

describe('a background refresh of a list with a query', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useJunkStore.setState({ verdictsByEmail: {}, loaded: {} });
    useEmailStore.getState().reset();
  });

  it('keeps a list longer than the refresh and adds what is new at the top', async () => {
    loaded(700);
    vi.mocked(api.listThreads).mockResolvedValue(page([row(-1), ...rows(0, 499)], cursorAt(498)));

    await refresh();

    expect(ids()).toHaveLength(701);
    expect(ids().slice(0, 2)).toEqual(['e-1', 'e0']);
    expect(useEmailStore.getState().paging.cursor).toEqual(cursorAt(699));
    expect(api.listThreads).toHaveBeenCalledWith('acc', 'invoice', 'inbox', false, null, 500, false);
  });

  it('does not change the list id, so a page in flight still lands', async () => {
    loaded(100);
    const before = useEmailStore.getState().currentFetchId;
    let finishPage: (value: ThreadPage) => void = () => {};
    vi.mocked(api.listThreads).mockReturnValueOnce(
      new Promise((resolve) => {
        finishPage = resolve;
      }),
    );
    const paging = useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    vi.mocked(api.listThreads).mockResolvedValueOnce(page(rows(0, 100), cursorAt(99)));
    await refresh();
    finishPage(page(rows(100, 150), cursorAt(149)));
    await paging;

    expect(useEmailStore.getState().currentFetchId).toBe(before);
    expect(ids()).toHaveLength(150);
  });

  it('drops a refresh that outlived its list', async () => {
    loaded(50);
    let finish: (value: ThreadPage) => void = () => {};
    vi.mocked(api.listThreads).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const refreshing = refresh();

    useEmailStore.setState({ currentFetchId: useEmailStore.getState().currentFetchId + 1, emails: [row(900)] });
    finish(page(rows(0, 50), cursorAt(49)));
    await refreshing;

    expect(ids()).toEqual(['e900']);
  });

  it('a newer refresh wins over an older one', async () => {
    loaded(50);
    let finishOld: (value: ThreadPage) => void = () => {};
    vi.mocked(api.listThreads).mockReturnValueOnce(
      new Promise((resolve) => {
        finishOld = resolve;
      }),
    );
    const older = refresh();
    vi.mocked(api.listThreads).mockResolvedValueOnce(page([row(-1), ...rows(0, 49)], cursorAt(48)));
    await refresh();

    finishOld(page(rows(0, 50), cursorAt(49)));
    await older;

    expect(ids()[0]).toBe('e-1');
  });

  it('a failed refresh leaves the list and says nothing on it', async () => {
    loaded(50);
    vi.mocked(api.listThreads).mockRejectedValue(new Error('offline'));

    await refresh();

    expect(ids()).toHaveLength(50);
    expect(useEmailStore.getState().error).toBeNull();
  });

  it('keeps Retry through a good refresh', async () => {
    loaded(50);
    useEmailStore.setState({ paging: { status: 'failed', cursor: cursorAt(49), offset: 0 } });
    vi.mocked(api.listThreads).mockResolvedValue(page([row(-1), ...rows(0, 49)], cursorAt(48)));

    await refresh();

    expect(useEmailStore.getState().paging.status).toBe('failed');
  });

  it('waits for the first page instead of racing it', async () => {
    useEmailStore.setState({ searchQuery: 'invoice', isLoading: true });

    await refresh();

    expect(api.listThreads).not.toHaveBeenCalled();
  });

  it('holds new rows until their verdicts are in', async () => {
    loaded(50);
    vi.mocked(api.listThreads).mockResolvedValue(page([row(-1), ...rows(0, 49)], cursorAt(48)));
    vi.mocked(api.getJunkVerdicts).mockRejectedValueOnce(new Error('no verdicts'));

    await refresh();

    expect(ids()).not.toContain('e-1');
  });
});

describe('a background refresh of a plain mailbox', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useJunkStore.setState({ verdictsByEmail: {}, loaded: {} });
    useEmailStore.getState().reset();
  });

  it('replaces the list and takes a new list id, as before', async () => {
    useEmailStore.setState({ emails: rows(0, 3), searchQuery: null });
    const before = useEmailStore.getState().currentFetchId;
    vi.mocked(api.getEmails).mockResolvedValue(rows(0, 2));
    vi.mocked(api.getEmailCount).mockResolvedValue(2);

    await refresh();

    expect(ids()).toEqual(['e0', 'e1']);
    expect(useEmailStore.getState().currentFetchId).toBe(before + 1);
    expect(api.listThreads).not.toHaveBeenCalled();
  });
});
