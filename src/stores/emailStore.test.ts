// Unit tests for emailStore pure helpers.
//
// No React, no Tauri, no Zustand — just plain function calls.

import { beforeEach, describe, expect, it } from 'vitest';

import { computeHasMore, useEmailStore } from './emailStore';

const PAGE_SIZE = 50;

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
