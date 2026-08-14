// Unit tests for emailStore pure helpers.
//
// No React, no Tauri, no Zustand — just plain function calls.

import { beforeEach, describe, expect, it } from 'vitest';

import type { Email } from '@/types';
import {
  appendUniqueEmails,
  computeHasMore,
  computeHasMoreAfterPage,
  mergeThreadRefresh,
  refetchLimit,
  removeEmailFromSlices,
  useEmailStore,
} from './emailStore';

const PAGE_SIZE = 50;

function makeEmail(id: string): Email {
  return { id } as Email;
}

describe('computeHasMore', () => {
  describe('with known totalCount (> 0)', () => {
    it('returns true when emails.length is below totalCount', () => {
      expect(computeHasMore(50, 200, PAGE_SIZE)).toBe(true);
    });

    it('returns false when emails.length equals totalCount', () => {
      expect(computeHasMore(200, 200, PAGE_SIZE)).toBe(false);
    });

    it('returns false when emails.length exceeds totalCount', () => {
      // Shouldn't happen in practice but the comparison must be strict <.
      expect(computeHasMore(250, 200, PAGE_SIZE)).toBe(false);
    });

    it('returns true even when a partial first page is below totalCount', () => {
      // Partial page can happen if the backend returns fewer than PAGE_SIZE
      // on the last page; the totalCount comparison still rules.
      expect(computeHasMore(7, 12, PAGE_SIZE)).toBe(true);
    });
  });

  describe('with unknown totalCount (-1) — filtered / search endpoints', () => {
    // Regression for the Globex bug: filter endpoint returned 473 thread reps
    // but only the first 50 were shown because hasMore was pinned false when
    // totalCount = -1. The fallback is "got a full page => assume there's more".
    it('returns true when the first page is full and totalCount is -1', () => {
      expect(computeHasMore(50, -1, PAGE_SIZE)).toBe(true);
    });

    it('returns false when the first page is short and totalCount is -1', () => {
      // Backend gave us fewer than PAGE_SIZE → that was the whole result set.
      expect(computeHasMore(12, -1, PAGE_SIZE)).toBe(false);
    });

    it('returns false on an empty result set', () => {
      expect(computeHasMore(0, -1, PAGE_SIZE)).toBe(false);
    });

    it('returns true when we have already loaded multiple full pages', () => {
      // After loadMore appended another full page (50 + 50 = 100), totalCount
      // is still -1 → moreEmails.length is the right signal for hasMore.
      // computeHasMore(moreEmails.length=50, -1) → true.
      expect(computeHasMore(50, -1, PAGE_SIZE)).toBe(true);
    });
  });

  describe('with totalCount = 0', () => {
    // 0 is a legitimate empty result for unfiltered queries (getEmailCount).
    // Treat it as a known total and avoid trying to load more.
    it('returns false when totalCount is 0 and no emails', () => {
      expect(computeHasMore(0, 0, PAGE_SIZE)).toBe(false);
    });

    it('returns false when totalCount is 0 even if emails were somehow returned', () => {
      // Defensive: shouldn't happen, but must not loop forever.
      expect(computeHasMore(5, 0, PAGE_SIZE)).toBe(false);
    });
  });

  describe('with a custom pageSize', () => {
    it('uses the provided pageSize for the unknown-total fallback', () => {
      expect(computeHasMore(20, -1, 20)).toBe(true);
      expect(computeHasMore(19, -1, 20)).toBe(false);
    });

    it('uses the default PAGE_SIZE when none is provided', () => {
      // Default is 50.
      expect(computeHasMore(50, -1)).toBe(true);
      expect(computeHasMore(49, -1)).toBe(false);
    });
  });
});

describe('computeHasMoreAfterPage', () => {
  it('stops when the page came back short, even if totalCount is larger', () => {
    // Regression: the Sent/Spam/Trash views list one mailbox but compare
    // against the account-wide inbox thread count. With 1 sent email and 32
    // inbox threads, hasMore stayed true forever and the list re-requested an
    // empty page on a loop, leaving "Loading more…" spinning.
    expect(computeHasMoreAfterPage(1, 0, 32, PAGE_SIZE)).toBe(false);
    expect(computeHasMoreAfterPage(12, 11, 999, PAGE_SIZE)).toBe(false);
  });

  it('keeps paging while the backend hands back full pages', () => {
    expect(computeHasMoreAfterPage(100, PAGE_SIZE, 200, PAGE_SIZE)).toBe(true);
  });

  it('stops once a full page reaches the known total', () => {
    expect(computeHasMoreAfterPage(200, PAGE_SIZE, 200, PAGE_SIZE)).toBe(false);
  });

  it('falls back to page fullness when the total is unknown', () => {
    expect(computeHasMoreAfterPage(50, PAGE_SIZE, -1, PAGE_SIZE)).toBe(true);
    expect(computeHasMoreAfterPage(12, 12, -1, PAGE_SIZE)).toBe(false);
  });
});

describe('appendUniqueEmails', () => {
  it('appends a non-overlapping page unchanged', () => {
    const existing = [makeEmail('a'), makeEmail('b')];
    const more = [makeEmail('c'), makeEmail('d')];
    expect(appendUniqueEmails(existing, more).map((e) => e.id)).toEqual(['a', 'b', 'c', 'd']);
  });

  it('drops emails whose id is already in the list', () => {
    // Regression: offset-based load-more re-returns a row when a new email is
    // inserted at the top (post-send sync). Without dedup, two list children
    // get the same React key. See appendUniqueEmails doc comment.
    const existing = [makeEmail('a'), makeEmail('b'), makeEmail('c')];
    const more = [makeEmail('c'), makeEmail('d')];
    expect(appendUniqueEmails(existing, more).map((e) => e.id)).toEqual(['a', 'b', 'c', 'd']);
  });

  it('drops the entire page when it fully overlaps', () => {
    const existing = [makeEmail('a'), makeEmail('b')];
    const more = [makeEmail('a'), makeEmail('b')];
    expect(appendUniqueEmails(existing, more).map((e) => e.id)).toEqual(['a', 'b']);
  });

  it('produces no duplicate ids', () => {
    const existing = [makeEmail('a'), makeEmail('b')];
    const more = [makeEmail('b'), makeEmail('c'), makeEmail('c')];
    const ids = appendUniqueEmails(existing, more).map((e) => e.id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});

describe('mergeThreadRefresh', () => {
  // A thread refresh (after a send, or after a sync batch replaces an
  // optimistic sent row with the provider's real copy) must not blank out
  // bodies the user already has expanded — getThread returns rows with empty
  // bodies (they load lazily).
  function threadEmail(id: string, timestamp: number, body = ''): Email {
    return { id, timestamp, body } as Email;
  }

  it('returns the fetched rows sorted oldest-first', () => {
    const fetched = [threadEmail('b', 200), threadEmail('a', 100)];
    expect(mergeThreadRefresh([], fetched).map((e) => e.id)).toEqual(['a', 'b']);
  });

  it('keeps an already-loaded body when the fetched row has an empty one', () => {
    const existing = [threadEmail('a', 100, '<p>loaded</p>')];
    const fetched = [threadEmail('a', 100), threadEmail('local-sent-1', 200, 'reply body')];
    const merged = mergeThreadRefresh(existing, fetched);
    expect(merged.find((e) => e.id === 'a')?.body).toBe('<p>loaded</p>');
    expect(merged.find((e) => e.id === 'local-sent-1')?.body).toBe('reply body');
  });

  it('prefers a non-empty fetched body over the cached one', () => {
    const existing = [threadEmail('a', 100, 'stale')];
    const fetched = [threadEmail('a', 100, 'fresh')];
    expect(mergeThreadRefresh(existing, fetched)[0].body).toBe('fresh');
  });

  it('drops rows that are no longer in the thread (reconciled synthetic row)', () => {
    const existing = [threadEmail('a', 100), threadEmail('local-sent-1', 200, 'reply')];
    const fetched = [threadEmail('a', 100), threadEmail('imap-real-7', 200, '')];
    expect(mergeThreadRefresh(existing, fetched).map((e) => e.id)).toEqual(['a', 'imap-real-7']);
  });
});

describe('removeEmailFromSlices', () => {
  // Shared by deleteEmail and moveEmail: both make the email vanish from its
  // current view (inbox list, thread view, open thread tabs, selection).
  function slices(selectedId: string | null) {
    const emails = [makeEmail('a'), makeEmail('b')];
    return {
      emails,
      threadEmails: [makeEmail('a')],
      selectedEmail: selectedId ? makeEmail(selectedId) : null,
      totalCount: 2,
      tabs: [{ type: 'thread' as const, threadEmails: [makeEmail('a'), makeEmail('b')] }],
    };
  }

  it('removes the email from every slice and decrements the count', () => {
    // biome-ignore lint/suspicious/noExplicitAny: minimal tab shape for a pure-helper test
    const next = removeEmailFromSlices(slices(null) as any, 'a');
    expect(next.emails.map((e) => e.id)).toEqual(['b']);
    expect(next.threadEmails).toEqual([]);
    expect(next.totalCount).toBe(1);
    expect((next.tabs[0] as { threadEmails: Email[] }).threadEmails.map((e) => e.id)).toEqual(['b']);
  });

  it('clears selectedEmail only when it is the removed email', () => {
    // biome-ignore lint/suspicious/noExplicitAny: minimal tab shape for a pure-helper test
    expect(removeEmailFromSlices(slices('a') as any, 'a').selectedEmail).toBeNull();
    // biome-ignore lint/suspicious/noExplicitAny: minimal tab shape for a pure-helper test
    expect(removeEmailFromSlices(slices('b') as any, 'a').selectedEmail?.id).toBe('b');
  });

  it('never lets the count go negative', () => {
    // biome-ignore lint/suspicious/noExplicitAny: minimal tab shape for a pure-helper test
    const state = { ...slices(null), totalCount: 0 } as any;
    expect(removeEmailFromSlices(state, 'a').totalCount).toBe(0);
  });
});

describe('sentRefreshTick', () => {
  beforeEach(() => useEmailStore.getState().reset());

  it('starts at 0 and increments on bumpSentRefresh', () => {
    expect(useEmailStore.getState().sentRefreshTick).toBe(0);
    useEmailStore.getState().bumpSentRefresh();
    useEmailStore.getState().bumpSentRefresh();
    expect(useEmailStore.getState().sentRefreshTick).toBe(2);
  });
});

describe('pendingChatDraft slot', () => {
  // The chat dispatcher sets this when a reply draft arrives so that
  // EmailView, on the next thread load, can open the inline ReplyCompose
  // with the AI body prepended to the quoted template — mirroring the
  // "click Reply on a thread" UX. Consume clears it so a later thread
  // load of the same email does not re-open a stale draft.
  beforeEach(() => useEmailStore.getState().reset());

  it('starts null', () => {
    expect(useEmailStore.getState().pendingChatDraft).toBeNull();
  });

  it('setPendingChatDraft stores the payload', () => {
    useEmailStore.getState().setPendingChatDraft({ emailId: 'e1', body: 'hello' });
    expect(useEmailStore.getState().pendingChatDraft).toEqual({ emailId: 'e1', body: 'hello' });
  });

  it('consumePendingChatDraft clears it', () => {
    useEmailStore.getState().setPendingChatDraft({ emailId: 'e1', body: 'hello' });
    useEmailStore.getState().consumePendingChatDraft();
    expect(useEmailStore.getState().pendingChatDraft).toBeNull();
  });

  it('reset clears pendingChatDraft', () => {
    useEmailStore.getState().setPendingChatDraft({ emailId: 'e1', body: 'hello' });
    useEmailStore.getState().reset();
    expect(useEmailStore.getState().pendingChatDraft).toBeNull();
  });
});

// A background refresh must not undo the user's paging. Every refetch starts at
// offset 0, so asking for a single page throws away every page the user had
// loaded — the list snaps back to 50 rows underneath whatever they were reading.
describe('refetchLimit', () => {
  const CAP = 500;

  it('asks for a full page when nothing is loaded yet', () => {
    expect(refetchLimit(0, PAGE_SIZE, CAP)).toBe(PAGE_SIZE);
  });

  it('never asks for less than a page', () => {
    expect(refetchLimit(12, PAGE_SIZE, CAP)).toBe(PAGE_SIZE);
  });

  it('asks for as many rows as the user had loaded', () => {
    expect(refetchLimit(250, PAGE_SIZE, CAP)).toBe(250);
  });

  it('stops at the cap so a very deep list does not re-query everything', () => {
    expect(refetchLimit(4000, PAGE_SIZE, CAP)).toBe(CAP);
  });
});
