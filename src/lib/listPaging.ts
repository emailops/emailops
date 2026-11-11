import type { Email, ThreadCursor } from '@/types';

/**
 * Where a list stands, as pure state. `idle` can load more, `exhausted` reached
 * the end, `failed` waits for Retry: scrolling alone never asks again.
 */
export type ListStatus = 'idle' | 'loading' | 'failed' | 'exhausted';

export interface ListPaging {
  status: ListStatus;
  /** Where the next page of a list with a query starts. `null` once it reached the end. */
  cursor: ThreadCursor | null;
  /** The next row number of a plain mailbox. */
  offset: number;
}

export const INITIAL_PAGING: ListPaging = { status: 'exhausted', cursor: null, offset: 0 };

export type ListPagingAction =
  | { type: 'reset' }
  | { type: 'requested' }
  | { type: 'cancelled' }
  | { type: 'pageLoaded'; cursor: ThreadCursor | null; offset: number; hasMore: boolean }
  | { type: 'failed' }
  | { type: 'retried' }
  | { type: 'refreshed'; cursor: ThreadCursor | null };

export function listPagingReducer(state: ListPaging, action: ListPagingAction): ListPaging {
  switch (action.type) {
    case 'reset':
      return INITIAL_PAGING;
    case 'requested':
      return state.status === 'idle' ? { ...state, status: 'loading' } : state;
    case 'cancelled':
      return state.status === 'loading' ? { ...state, status: 'idle' } : state;
    case 'pageLoaded':
      return { status: action.hasMore ? 'idle' : 'exhausted', cursor: action.cursor, offset: action.offset };
    case 'failed':
      return { ...state, status: 'failed' };
    case 'retried':
      return state.status === 'failed' ? { ...state, status: 'idle' } : state;
    case 'refreshed':
      // A good refresh does not clear a failure: Retry would blink through a sync.
      if (state.status === 'failed' || state.status === 'loading') return { ...state, cursor: action.cursor };
      return { ...state, status: action.cursor ? 'idle' : 'exhausted', cursor: action.cursor };
  }
}

export const selectHasMore = (paging: ListPaging): boolean => paging.status !== 'exhausted';
export const selectIsLoadingMore = (paging: ListPaging): boolean => paging.status === 'loading';
export const selectPageFailed = (paging: ListPaging): boolean => paging.status === 'failed';

/** A list with a query has one row per conversation, in one account. */
export function threadRowKey(email: Email): string {
  return `${email.accountId}\u0000${email.threadId}`;
}

/** A row's place in the list's order, the same shape as the cursor. */
export function sortKeyOf(email: Email): ThreadCursor {
  return { threadTs: email.threadLatestAt ?? email.timestamp, timestamp: email.timestamp, id: email.id };
}

/** Negative when `a` sorts below `b`: the list runs from the highest key down.
 *  Ids are ASCII, so this agrees with SQLite's binary order. */
export function compareCursor(a: ThreadCursor, b: ThreadCursor): number {
  if (a.threadTs !== b.threadTs) return a.threadTs - b.threadTs;
  if (a.timestamp !== b.timestamp) return a.timestamp - b.timestamp;
  if (a.id === b.id) return 0;
  return a.id < b.id ? -1 : 1;
}

/** The cursor further down the list. `null` means the end, the lowest of all. */
export function lowerCursor(a: ThreadCursor | null, b: ThreadCursor | null): ThreadCursor | null {
  if (a === null || b === null) return null;
  return compareCursor(a, b) <= 0 ? a : b;
}

export interface ListHead {
  emails: Email[];
  nextCursor: ThreadCursor | null;
}

/**
 * Combine a refresh's first rows with a longer loaded list.
 *
 * The cut is the head's cursor, compared by sort key. A shared row would not
 * do: a deep row that got a reply sits in the head, and cutting there would
 * drop every row that was above it, for good, since the cursor stays below.
 *
 * Without a row unchanged in both, the two ends may not meet and rows could
 * hide between them. The list goes back to the head, and the rest return on
 * the way down.
 */
export function mergeHead(
  loaded: Email[],
  cursor: ThreadCursor | null,
  head: ListHead,
): { emails: Email[]; cursor: ThreadCursor | null } {
  const { emails: headRows, nextCursor } = head;
  if (nextCursor === null) return { emails: headRows, cursor: null };

  const loadedKeys = new Map(loaded.map((email) => [threadRowKey(email), sortKeyOf(email)]));
  const overlaps = headRows.some((email) => {
    const key = loadedKeys.get(threadRowKey(email));
    return key !== undefined && compareCursor(key, sortKeyOf(email)) === 0;
  });
  if (!overlaps) return { emails: headRows, cursor: nextCursor };

  const headKeys = new Set(headRows.map(threadRowKey));
  const tail = loaded.filter(
    (email) => !headKeys.has(threadRowKey(email)) && compareCursor(sortKeyOf(email), nextCursor) < 0,
  );
  return { emails: [...headRows, ...tail], cursor: lowerCursor(cursor, nextCursor) };
}
