import { describe, expect, it } from 'vitest';
import type { Email, ThreadCursor } from '@/types';
import {
  INITIAL_PAGING,
  type ListPaging,
  listPagingReducer,
  lowerCursor,
  mergeHead,
  selectHasMore,
  selectIsLoadingMore,
  selectPageFailed,
  sortKeyOf,
} from './listPaging';

const CURSOR: ThreadCursor = { threadTs: 100, timestamp: 90, id: 'b' };
const idle: ListPaging = { status: 'idle', cursor: CURSOR, offset: 50 };

/** Rows numbered from the top: row 0 sorts first. */
function row(n: number, threadTs = 10_000 - n): Email {
  return {
    id: `e${n}`,
    accountId: 'a',
    threadId: `t${n}`,
    timestamp: threadTs,
    threadLatestAt: threadTs,
  } as Email;
}

function rows(from: number, to: number): Email[] {
  return Array.from({ length: to - from }, (_, i) => row(from + i));
}

const ids = (emails: Email[]) => emails.map((e) => e.id);

describe('listPagingReducer', () => {
  it('moves through a page', () => {
    const loading = listPagingReducer(idle, { type: 'requested' });
    expect(loading.status).toBe('loading');
    const done = listPagingReducer(loading, { type: 'pageLoaded', cursor: null, offset: 100, hasMore: false });
    expect(done).toEqual({ status: 'exhausted', cursor: null, offset: 100 });
  });

  it('asks for one page at a time, and nothing after the end or a failure', () => {
    for (const status of ['loading', 'failed', 'exhausted'] as const) {
      const state = { ...idle, status };
      expect(listPagingReducer(state, { type: 'requested' })).toBe(state);
    }
  });

  it('a page dropped on arrival frees the list', () => {
    const loading = listPagingReducer(idle, { type: 'requested' });
    expect(listPagingReducer(loading, { type: 'cancelled' }).status).toBe('idle');
    expect(listPagingReducer(INITIAL_PAGING, { type: 'cancelled' })).toBe(INITIAL_PAGING);
  });

  it('only a retry leaves a failure', () => {
    const failed = listPagingReducer(idle, { type: 'failed' });
    expect(failed).toEqual({ ...idle, status: 'failed' });
    expect(listPagingReducer(failed, { type: 'retried' }).status).toBe('idle');
    expect(listPagingReducer(idle, { type: 'retried' })).toBe(idle);
  });

  it('a refresh keeps a failure and a page in flight', () => {
    const next = { ...CURSOR, id: 'a' };
    for (const status of ['failed', 'loading'] as const) {
      expect(listPagingReducer({ ...idle, status }, { type: 'refreshed', cursor: next })).toEqual({
        ...idle,
        status,
        cursor: next,
      });
    }
  });

  it('a refresh without overlap reopens a list that had ended', () => {
    const ended: ListPaging = { status: 'exhausted', cursor: null, offset: 0 };
    expect(listPagingReducer(ended, { type: 'refreshed', cursor: CURSOR }).status).toBe('idle');
    expect(listPagingReducer(idle, { type: 'refreshed', cursor: null }).status).toBe('exhausted');
  });

  it('a reset forgets the list', () => {
    expect(listPagingReducer(idle, { type: 'reset' })).toBe(INITIAL_PAGING);
  });

  it('exposes its state through selectors', () => {
    expect(selectHasMore(idle)).toBe(true);
    expect(selectHasMore(INITIAL_PAGING)).toBe(false);
    expect(selectIsLoadingMore({ ...idle, status: 'loading' })).toBe(true);
    expect(selectPageFailed({ ...idle, status: 'failed' })).toBe(true);
    expect(selectHasMore({ ...idle, status: 'failed' })).toBe(true);
  });
});

describe('lowerCursor', () => {
  const higher = { threadTs: 200, timestamp: 1, id: 'z' };
  const sameSecond = { ...CURSOR, id: 'a' };

  it('takes the one further down the list', () => {
    expect(lowerCursor(CURSOR, higher)).toBe(CURSOR);
    expect(lowerCursor(higher, CURSOR)).toBe(CURSOR);
    expect(lowerCursor(CURSOR, sameSecond)).toBe(sameSecond);
  });

  it('treats the end of the list as the lowest of all', () => {
    expect(lowerCursor(null, CURSOR)).toBeNull();
    expect(lowerCursor(CURSOR, null)).toBeNull();
  });
});

describe('mergeHead', () => {
  const cursorAt = (n: number) => sortKeyOf(row(n));

  it('takes a head with no cursor as the whole list', () => {
    const merged = mergeHead(rows(0, 900), cursorAt(899), { emails: rows(0, 40), nextCursor: null });
    expect(ids(merged.emails)).toEqual(ids(rows(0, 40)));
    expect(merged.cursor).toBeNull();
  });

  it('keeps a long list when new rows come in at the top', () => {
    const fresh = [row(-2), row(-1)];
    const head = { emails: [...fresh, ...rows(0, 498)], nextCursor: cursorAt(497) };

    const merged = mergeHead(rows(0, 900), cursorAt(899), head);

    expect(merged.emails).toHaveLength(902);
    expect(ids(merged.emails.slice(0, 3))).toEqual(['e-2', 'e-1', 'e0']);
    expect(merged.cursor).toEqual(cursorAt(899));
  });

  it('a deep row that rises to the head costs no row that was above it', () => {
    const risen = row(700, 20_000);
    const head = { emails: [risen, ...rows(0, 499)], nextCursor: cursorAt(498) };

    const merged = mergeHead(rows(0, 900), cursorAt(899), head);

    expect(merged.emails).toHaveLength(900);
    expect(new Set(ids(merged.emails)).size).toBe(900);
    expect(ids(merged.emails).slice(0, 2)).toEqual(['e700', 'e0']);
    expect(ids(merged.emails).slice(500, 502)).toEqual(['e499', 'e500']);
    expect(merged.cursor).toEqual(cursorAt(899));
  });

  it('a row that stopped matching leaves without reordering the rest', () => {
    // "Only unread" with 41-50 read: the head skips them and reaches further.
    const head = { emails: [...rows(0, 40), ...rows(50, 60)], nextCursor: cursorAt(59) };

    const merged = mergeHead(rows(0, 50), cursorAt(49), head);

    expect(ids(merged.emails)).toEqual(ids(head.emails));
    expect(merged.cursor).toEqual(cursorAt(59));
  });

  it('goes back to the head when nothing overlaps', () => {
    const head = { emails: rows(-60, -10), nextCursor: cursorAt(-11) };

    const merged = mergeHead(rows(0, 50), cursorAt(49), head);

    expect(ids(merged.emails)).toEqual(ids(head.emails));
    expect(merged.cursor).toEqual(cursorAt(-11));
  });

  it('a row that only moved up does not count as overlap', () => {
    const risen = row(30, 20_000);
    const head = { emails: [risen, ...rows(-49, 0)], nextCursor: cursorAt(-1) };

    const merged = mergeHead(rows(0, 50), cursorAt(49), head);

    expect(merged.emails).toHaveLength(50);
    expect(merged.cursor).toEqual(cursorAt(-1));
  });

  it('a list that reached the end stays there', () => {
    const fresh = row(-1);
    const head = { emails: [fresh, ...rows(0, 49)], nextCursor: cursorAt(48) };

    const merged = mergeHead(rows(0, 60), null, head);

    expect(merged.emails).toHaveLength(61);
    expect(merged.cursor).toBeNull();
  });

  it('two accounts can hold the same thread id', () => {
    const mine = { ...row(5), accountId: 'b', threadId: 't0' };
    const head = { emails: rows(0, 2), nextCursor: cursorAt(1) };

    const merged = mergeHead([...rows(0, 2), mine], cursorAt(1), head);

    expect(merged.emails.filter((e) => e.threadId === 't0')).toHaveLength(2);
  });
});
