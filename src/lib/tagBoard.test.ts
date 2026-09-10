import { describe, expect, it } from 'vitest';
import type { Email, TagStat } from '@/types';
import {
  applySavedOrder,
  columnKey,
  DENSITY_MIN_COLUMN_PX,
  dedupeOrdinalThreads,
  fitsInOneRow,
  initialTagBoardState,
  isTagBoardDensity,
  isTagBoardType,
  mapWithConcurrency,
  moveColumnKey,
  normaliseCustomRange,
  planColumnLoads,
  rangeToWindow,
  selectColumn,
  selectIsEmpty,
  selectNextPageOffset,
  selectRenderableColumns,
  senderLabel,
  TAG_BOARD_PAGE_SIZE,
  type TagBoardRange,
  type TagBoardState,
  tagBoardReducer,
} from './tagBoard';

function email(id: string, overrides: Partial<Email> = {}): Email {
  return {
    id,
    accountId: 'acc',
    threadId: `thread-${id}`,
    messageId: null,
    subject: `Subject ${id}`,
    sender: 'Sender Name',
    senderEmail: 'sender@example.com',
    recipients: [],
    cc: [],
    body: '',
    snippet: 'snippet',
    timestamp: 1000,
    isRead: false,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
    ...overrides,
  };
}

const ACC = 'acc-1';

function stat(tagValue: string, count: number, accountId: string = ACC): TagStat {
  return { accountId, tagValue, count, sentShare: 0, readShare: 0, lastActivityAt: null, score: 0 };
}

/** Column key for a tag on the default test account. */
function key(tagValue: string, accountId: string = ACC): string {
  return columnKey(accountId, tagValue);
}

/** State with one loaded column, the common starting point below. */
function stateWithColumns(stats: TagStat[], queryKey = 'q'): TagBoardState {
  return tagBoardReducer(initialTagBoardState, { type: 'COLUMNS_LOADED', stats, queryKey });
}

describe('isTagBoardType', () => {
  it('accepts the four classified tag types', () => {
    expect(isTagBoardType('company')).toBe(true);
    expect(isTagBoardType('priority')).toBe(true);
    expect(isTagBoardType('intent')).toBe(true);
    expect(isTagBoardType('topic')).toBe(true);
  });

  it('rejects a tag type the board cannot render', () => {
    expect(isTagBoardType('junk')).toBe(false);
  });

  it('rejects a non-string persisted preference', () => {
    expect(isTagBoardType(null)).toBe(false);
    expect(isTagBoardType(42)).toBe(false);
  });
});

describe('tagBoardReducer — COLUMNS_LOADED', () => {
  it('builds one column per tag value, in the order the stats arrive', () => {
    const state = stateWithColumns([stat('globex', 12), stat('initech', 5)]);

    expect(state.columns.map((c) => c.value)).toEqual(['globex', 'initech']);
    expect(state.columns[0].threadCount).toBe(12);
  });

  it('starts every column empty and not yet loaded', () => {
    const state = stateWithColumns([stat('globex', 12)]);

    expect(state.columns[0].emails).toEqual([]);
    expect(state.columns[0].isLoading).toBe(false);
    expect(state.columns[0].hasMore).toBe(true);
  });

  it('drops emails loaded under a different query when the window changes', () => {
    // Changing the range/category re-runs the stats query. The blocks can come
    // back identical, but their rows were fetched under the OLD window — the
    // "Today shows yesterday's mail" bug. Emails survive only a same-query
    // refresh.
    let state = stateWithColumns([stat('globex', 12)], 'q-all-time');
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    state = tagBoardReducer(state, {
      type: 'COLUMNS_LOADED',
      stats: [stat('globex', 3)],
      queryKey: 'q-today',
    });

    expect(selectColumn(state, key('globex'))?.emails).toEqual([]);
    expect(selectColumn(state, key('globex'))?.threadCount).toBe(3);
  });

  it('keeps emails already loaded for a column that survives a refresh', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    // A refresh re-reads the stats; the count moved but the column is the same.
    state = tagBoardReducer(state, { type: 'COLUMNS_LOADED', stats: [stat('globex', 13)], queryKey: 'q' });

    expect(state.columns[0].emails.map((e) => e.id)).toEqual(['e1']);
    expect(state.columns[0].threadCount).toBe(13);
  });

  it('drops emails for a column that disappeared from the stats', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });
    state = tagBoardReducer(state, { type: 'COLUMNS_LOADED', stats: [stat('initech', 4)], queryKey: 'q' });

    expect(state.columns.map((c) => c.value)).toEqual(['initech']);
    expect(state.columns[0].emails).toEqual([]);
  });

  it('clears a previous board-level error', () => {
    let state = tagBoardReducer(initialTagBoardState, { type: 'COLUMNS_ERROR', error: 'boom' });
    state = tagBoardReducer(state, { type: 'COLUMNS_LOADED', stats: [stat('globex', 1)], queryKey: 'q' });

    expect(state.error).toBeNull();
    expect(state.isLoadingColumns).toBe(false);
  });
});

describe('tagBoardReducer — SET_TAG_TYPE', () => {
  it('switches the dimension and drops the previous columns', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, { type: 'SET_TAG_TYPE', tagType: 'topic' });

    expect(state.tagType).toBe('topic');
    expect(state.columns).toEqual([]);
  });

  it('is a no-op when the dimension is already selected', () => {
    const state = stateWithColumns([stat('globex', 12)]);
    const next = tagBoardReducer(state, { type: 'SET_TAG_TYPE', tagType: 'company' });

    expect(next).toBe(state);
  });
});

describe('tagBoardReducer — page loading', () => {
  it('appends a loaded page to the column it belongs to', () => {
    let state = stateWithColumns([stat('globex', 12), stat('initech', 5)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1'), email('e2')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e3')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    expect(selectColumn(state, key('globex'))?.emails.map((e) => e.id)).toEqual(['e1', 'e2', 'e3']);
    expect(selectColumn(state, key('initech'))?.emails).toEqual([]);
  });

  it('drops emails already in the column instead of appending them twice', () => {
    // React StrictMode double-invokes effects in dev, and a refresh can race a
    // page already in flight. Either way the same page can arrive twice; when
    // it did, the duplicate ids produced duplicate React keys and the board
    // rendered phantom blank cards between the real ones.
    let state = stateWithColumns([stat('globex', 12)]);
    const page = [email('e1'), email('e2')];

    state = tagBoardReducer(state, { type: 'PAGE_LOADED', key: key('globex'), emails: page, pageSize: 8 });
    state = tagBoardReducer(state, { type: 'PAGE_LOADED', key: key('globex'), emails: page, pageSize: 8 });

    expect(selectColumn(state, key('globex'))?.emails.map((e) => e.id)).toEqual(['e1', 'e2']);
  });

  it('keeps the new emails from a page that only partly overlaps', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1'), email('e2')],
      pageSize: 8,
    });
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e2'), email('e3')],
      pageSize: 8,
    });

    expect(selectColumn(state, key('globex'))?.emails.map((e) => e.id)).toEqual(['e1', 'e2', 'e3']);
  });

  it('keys dedup per account so the same thread id in two accounts both show', () => {
    // Under the unified scope two accounts can carry the same provider id;
    // they are distinct rows and must not collapse into one.
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('shared', { accountId: 'acc-a' }), email('shared', { accountId: 'acc-b' })],
      pageSize: 8,
    });

    expect(selectColumn(state, key('globex'))?.emails).toHaveLength(2);
  });

  it('ignores a page for a column that is no longer on the board', () => {
    const state = stateWithColumns([stat('globex', 12)]);
    const next = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('gone'),
      emails: [email('e1')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    expect(next.columns).toEqual(state.columns);
  });

  it('clears hasMore when a short page comes back', () => {
    // total_count is always -1 for tag filters, so a page shorter than the
    // requested size is the only end-of-list signal there is.
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1')],
      pageSize: 8,
    });

    expect(selectColumn(state, key('globex'))?.hasMore).toBe(false);
  });

  it('keeps hasMore when a full page comes back', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1'), email('e2')],
      pageSize: 2,
    });

    expect(selectColumn(state, key('globex'))?.hasMore).toBe(true);
  });

  it('clears the column error and loading flag on a successful page', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, { type: 'PAGE_ERROR', key: key('globex'), error: 'boom' });
    state = tagBoardReducer(state, { type: 'PAGE_LOADING', key: key('globex') });
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    expect(selectColumn(state, key('globex'))?.error).toBeNull();
    expect(selectColumn(state, key('globex'))?.isLoading).toBe(false);
  });

  it('records a page error against its own column only', () => {
    let state = stateWithColumns([stat('globex', 12), stat('initech', 5)]);
    state = tagBoardReducer(state, { type: 'PAGE_LOADING', key: key('globex') });
    state = tagBoardReducer(state, { type: 'PAGE_ERROR', key: key('globex'), error: 'boom' });

    expect(selectColumn(state, key('globex'))?.error).toBe('boom');
    expect(selectColumn(state, key('globex'))?.isLoading).toBe(false);
    expect(selectColumn(state, key('initech'))?.error).toBeNull();
  });

  it('stops offering more after a failed page so the board cannot spin forever', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    state = tagBoardReducer(state, { type: 'PAGE_ERROR', key: key('globex'), error: 'boom' });

    expect(selectColumn(state, key('globex'))?.hasMore).toBe(false);
  });
});

describe('selectNextPageOffset', () => {
  it('is the number of emails already loaded in the column', () => {
    let state = stateWithColumns([stat('globex', 12)]);
    expect(selectNextPageOffset(state, key('globex'))).toBe(0);

    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1'), email('e2')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    expect(selectNextPageOffset(state, key('globex'))).toBe(2);
  });

  it('is 0 for a column the board does not have', () => {
    expect(selectNextPageOffset(initialTagBoardState, key('gone'))).toBe(0);
  });
});

describe('selectIsEmpty', () => {
  it('is false while the columns are still loading', () => {
    const state = tagBoardReducer(initialTagBoardState, { type: 'COLUMNS_LOADING' });
    expect(selectIsEmpty(state)).toBe(false);
  });

  it('is false when the stats query failed — that is an error, not an empty board', () => {
    const state = tagBoardReducer(initialTagBoardState, { type: 'COLUMNS_ERROR', error: 'boom' });
    expect(selectIsEmpty(state)).toBe(false);
  });

  it('is true once a successful load returns no tags at all', () => {
    const state = stateWithColumns([]);
    expect(selectIsEmpty(state)).toBe(true);
  });

  it('is false once there is at least one column', () => {
    expect(selectIsEmpty(stateWithColumns([stat('globex', 1)]))).toBe(false);
  });
});

describe('mapWithConcurrency', () => {
  it('resolves results in input order regardless of completion order', async () => {
    const delays = [30, 0, 10];
    const results = await mapWithConcurrency(
      delays,
      2,
      (ms) => new Promise<number>((resolve) => setTimeout(() => resolve(ms), ms)),
    );

    expect(results).toEqual([30, 0, 10]);
  });

  it('never runs more than `limit` tasks at once', async () => {
    let running = 0;
    let peak = 0;

    await mapWithConcurrency([1, 2, 3, 4, 5, 6], 2, async () => {
      running += 1;
      peak = Math.max(peak, running);
      await new Promise((resolve) => setTimeout(resolve, 5));
      running -= 1;
    });

    expect(peak).toBe(2);
  });

  it('runs every task even when one rejects, and reports the rejection', async () => {
    // One failing column must not strand the rest of the board unloaded.
    const started: number[] = [];
    const results = await mapWithConcurrency([1, 2, 3], 2, async (n) => {
      started.push(n);
      if (n === 2) throw new Error('boom');
      return n;
    });

    expect(started.sort()).toEqual([1, 2, 3]);
    expect(results[0]).toBe(1);
    expect(results[1]).toBeInstanceOf(Error);
    expect(results[2]).toBe(3);
  });

  it('returns an empty array for no items', async () => {
    expect(await mapWithConcurrency([], 4, async (n) => n)).toEqual([]);
  });
});

describe('columnKey', () => {
  it('combines account and tag so one tag makes one block per account', () => {
    expect(columnKey('acc-a', 'globex')).not.toBe(columnKey('acc-b', 'globex'));
  });

  it('is stable for the same pair', () => {
    expect(columnKey('acc-a', 'globex')).toBe(columnKey('acc-a', 'globex'));
  });

  it('does not collide when a tag value contains the separator', () => {
    // Tag values are free text from the classifier; a naive "a:b" join would
    // let ("acc", "x::y") and ("acc::x", "y") land on the same block.
    expect(columnKey('acc', 'x::y')).not.toBe(columnKey('acc::x', 'y'));
  });
});

describe('rangeToWindow', () => {
  // Fixed local reference point: 2026-09-10 14:30 local time.
  const now = new Date(2026, 8, 10, 14, 30, 0);
  const startOf = (y: number, m: number, d: number) => Math.floor(new Date(y, m, d).getTime() / 1000);

  it('returns an open window for "all"', () => {
    expect(rangeToWindow('all', now)).toEqual({});
  });

  it('covers midnight-to-now for "today"', () => {
    const w = rangeToWindow('today', now);
    expect(w.since).toBe(startOf(2026, 8, 10));
    expect(w.until).toBeUndefined();
  });

  it('covers exactly the previous day for "yesterday"', () => {
    // Half-open: `until` is yesterday's end == today's start, so a thread at
    // 00:00:00 today belongs to Today and never to both.
    const w = rangeToWindow('yesterday', now);
    expect(w.since).toBe(startOf(2026, 8, 9));
    expect(w.until).toBe(startOf(2026, 8, 10));
  });

  it('covers the last 7 days including today for "last7"', () => {
    const w = rangeToWindow('last7', now);
    expect(w.since).toBe(startOf(2026, 8, 4));
    expect(w.until).toBeUndefined();
  });

  it('uses the supplied dates for "custom"', () => {
    const w = rangeToWindow('custom', now, { from: '2026-08-01', to: '2026-08-03' });
    expect(w.since).toBe(startOf(2026, 7, 1));
    // `to` is inclusive to the user, so the exclusive bound is the next midnight.
    expect(w.until).toBe(startOf(2026, 7, 4));
  });

  it('treats a custom range with no dates as unbounded', () => {
    expect(rangeToWindow('custom', now, { from: '', to: '' })).toEqual({});
  });

  it('normalises a reversed custom range instead of dropping a bound', () => {
    // Two date boxes are easy to fill in the wrong order, and silently
    // honouring only one of them showed mail from outside the range the user
    // could see on screen. Swap, the way every date picker does.
    const reversed = rangeToWindow('custom', now, { from: '2026-09-10', to: '2026-06-01' });
    const intended = rangeToWindow('custom', now, { from: '2026-06-01', to: '2026-09-10' });
    expect(reversed).toEqual(intended);
  });

  it('keeps both bounds when a range is reversed', () => {
    const w = rangeToWindow('custom', now, { from: '2026-08-10', to: '2026-08-01' });
    expect(w.since).toBe(startOf(2026, 7, 1));
    expect(w.until).toBe(startOf(2026, 7, 11));
  });

  it('still ignores a bound that has not been filled in yet', () => {
    // Mid-typing there is only one date; that is a single bound, not a range.
    expect(rangeToWindow('custom', now, { from: '2026-06-01', to: '' }).until).toBeUndefined();
    expect(rangeToWindow('custom', now, { from: '', to: '2026-06-01' }).since).toBeUndefined();
  });

  it('produces windows a later range never widens', () => {
    const ranges: TagBoardRange[] = ['today', 'yesterday', 'last7'];
    for (const r of ranges) {
      const w = rangeToWindow(r, now);
      expect(w.since).toBeLessThanOrEqual(Math.floor(now.getTime() / 1000));
    }
  });
});

describe('tagBoardReducer — per-account blocks', () => {
  it('makes one block per account for a tag both accounts carry', () => {
    const state = tagBoardReducer(initialTagBoardState, {
      type: 'COLUMNS_LOADED',
      queryKey: 'q',
      stats: [
        {
          accountId: 'acc-a',
          tagValue: 'globex',
          count: 4,
          sentShare: 0,
          readShare: 0,
          lastActivityAt: null,
          score: 0,
        },
        {
          accountId: 'acc-b',
          tagValue: 'globex',
          count: 2,
          sentShare: 0,
          readShare: 0,
          lastActivityAt: null,
          score: 0,
        },
      ],
    });

    expect(state.columns).toHaveLength(2);
    expect(state.columns.map((c) => c.accountId)).toEqual(['acc-a', 'acc-b']);
    expect(state.columns.every((c) => c.value === 'globex')).toBe(true);
  });

  it('pages the two blocks independently', () => {
    let state = tagBoardReducer(initialTagBoardState, {
      type: 'COLUMNS_LOADED',
      queryKey: 'q',
      stats: [
        {
          accountId: 'acc-a',
          tagValue: 'globex',
          count: 4,
          sentShare: 0,
          readShare: 0,
          lastActivityAt: null,
          score: 0,
        },
        {
          accountId: 'acc-b',
          tagValue: 'globex',
          count: 2,
          sentShare: 0,
          readShare: 0,
          lastActivityAt: null,
          score: 0,
        },
      ],
    });
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: columnKey('acc-a', 'globex'),
      emails: [email('e1', { accountId: 'acc-a' })],
      pageSize: 8,
    });

    expect(selectColumn(state, columnKey('acc-a', 'globex'))?.emails).toHaveLength(1);
    expect(selectColumn(state, columnKey('acc-b', 'globex'))?.emails).toHaveLength(0);
  });
});

describe('fitsInOneRow', () => {
  // The grid is `repeat(auto-fill, minmax(MIN, 1fr))` with a fixed gap, so the
  // number of columns the browser will lay out is derivable.
  it('is true when every block fits across the pane', () => {
    // 900px wide, 272px min column, 12px gap → 3 columns.
    expect(fitsInOneRow(3, 900, 272, 12)).toBe(true);
  });

  it('is false as soon as one block wraps to a second row', () => {
    expect(fitsInOneRow(4, 900, 272, 12)).toBe(false);
  });

  it('treats a pane too narrow for two columns as one column', () => {
    expect(fitsInOneRow(1, 300, 272, 12)).toBe(true);
    expect(fitsInOneRow(2, 300, 272, 12)).toBe(false);
  });

  it('is true for an empty board rather than dividing by zero', () => {
    expect(fitsInOneRow(0, 900, 272, 12)).toBe(true);
  });

  it('is true before the pane has been measured', () => {
    // Width 0 on first paint: assume one row so the board does not flash a
    // squashed layout before the ResizeObserver reports.
    expect(fitsInOneRow(5, 0, 272, 12)).toBe(true);
  });
});

describe('selectRenderableColumns', () => {
  it('keeps a block that has not been paged yet', () => {
    // Before its first page lands a block is unknown, not empty — hiding it
    // would make the board flicker blocks in one at a time.
    const state = stateWithColumns([stat('globex', 4)]);
    expect(selectRenderableColumns(state.columns)).toHaveLength(1);
  });

  it('drops a block whose page came back with nothing', () => {
    let state = stateWithColumns([stat('globex', 4), stat('initech', 2)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    expect(selectRenderableColumns(state.columns).map((c) => c.value)).toEqual(['initech']);
  });

  it('keeps a block that loaded rows', () => {
    let state = stateWithColumns([stat('globex', 4)]);
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: key('globex'),
      emails: [email('e1')],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    expect(selectRenderableColumns(state.columns)).toHaveLength(1);
  });

  it('keeps an errored block so its retry stays reachable', () => {
    let state = stateWithColumns([stat('globex', 4)]);
    state = tagBoardReducer(state, { type: 'PAGE_ERROR', key: key('globex'), error: 'boom' });

    expect(selectRenderableColumns(state.columns)).toHaveLength(1);
  });

  it('keeps a block that is mid-load', () => {
    let state = stateWithColumns([stat('globex', 4)]);
    state = tagBoardReducer(state, { type: 'PAGE_LOADING', key: key('globex') });

    expect(selectRenderableColumns(state.columns)).toHaveLength(1);
  });
});

describe('applySavedOrder', () => {
  const cols = (...values: string[]) =>
    tagBoardReducer(initialTagBoardState, {
      type: 'COLUMNS_LOADED',
      queryKey: 'q',
      stats: values.map((v) => stat(v, 1)),
    }).columns;

  it('returns the incoming ranking when nothing has been saved', () => {
    const c = cols('a', 'b', 'c');
    expect(applySavedOrder(c, []).map((x) => x.value)).toEqual(['a', 'b', 'c']);
  });

  it('honours the saved order', () => {
    const c = cols('a', 'b', 'c');
    expect(applySavedOrder(c, [key('c'), key('a'), key('b')]).map((x) => x.value)).toEqual(['c', 'a', 'b']);
  });

  it('appends blocks the saved order has never seen, in ranked order', () => {
    // A newly classified tag must not vanish just because the user has
    // reordered the board before it existed.
    const c = cols('a', 'b', 'new1', 'new2');
    expect(applySavedOrder(c, [key('b'), key('a')]).map((x) => x.value)).toEqual(['b', 'a', 'new1', 'new2']);
  });

  it('ignores saved keys whose block is gone', () => {
    const c = cols('a', 'b');
    expect(applySavedOrder(c, [key('gone'), key('b'), key('a')]).map((x) => x.value)).toEqual(['b', 'a']);
  });

  it('does not drop or duplicate any block', () => {
    const c = cols('a', 'b', 'c', 'd');
    const out = applySavedOrder(c, [key('d'), key('gone'), key('b')]);
    expect(out).toHaveLength(4);
    expect(new Set(out.map((x) => x.key)).size).toBe(4);
  });
});

describe('moveColumnKey', () => {
  // The drop indicator names a gap, so the caller says which side of the
  // target block the dragged one lands on.
  it('drops into the gap before the target', () => {
    expect(moveColumnKey(['a', 'b', 'c', 'd'], 'd', 'b', 'before')).toEqual(['a', 'd', 'b', 'c']);
  });

  it('drops into the gap after the target', () => {
    expect(moveColumnKey(['a', 'b', 'c', 'd'], 'd', 'b', 'after')).toEqual(['a', 'b', 'd', 'c']);
  });

  it('moves a block forwards into the gap after the target', () => {
    expect(moveColumnKey(['a', 'b', 'c', 'd'], 'a', 'c', 'after')).toEqual(['b', 'c', 'a', 'd']);
  });

  it('moves a block forwards into the gap before the target', () => {
    expect(moveColumnKey(['a', 'b', 'c', 'd'], 'a', 'c', 'before')).toEqual(['b', 'a', 'c', 'd']);
  });

  it('lands in the same place whichever side of the neighbouring gap is named', () => {
    // The gap between b and c is one gap: "after b" and "before c" mean it.
    expect(moveColumnKey(['a', 'b', 'c', 'd'], 'a', 'b', 'after')).toEqual(
      moveColumnKey(['a', 'b', 'c', 'd'], 'a', 'c', 'before'),
    );
  });

  it('is a no-op when dropped on itself', () => {
    const order = ['a', 'b', 'c'];
    expect(moveColumnKey(order, 'b', 'b', 'before')).toEqual(order);
    expect(moveColumnKey(order, 'b', 'b', 'after')).toEqual(order);
  });

  it('is a no-op for a key that is not in the order', () => {
    const order = ['a', 'b', 'c'];
    expect(moveColumnKey(order, 'zz', 'b', 'before')).toEqual(order);
    expect(moveColumnKey(order, 'a', 'zz', 'before')).toEqual(order);
  });

  it('preserves every key', () => {
    const out = moveColumnKey(['a', 'b', 'c', 'd'], 'c', 'a', 'before');
    expect([...out].sort()).toEqual(['a', 'b', 'c', 'd']);
  });
});

describe('isTagBoardDensity', () => {
  it('accepts the two layouts', () => {
    expect(isTagBoardDensity('granular')).toBe(true);
    expect(isTagBoardDensity('extended')).toBe(true);
  });

  it('rejects anything else, including a non-string pref', () => {
    expect(isTagBoardDensity('wide')).toBe(false);
    expect(isTagBoardDensity(null)).toBe(false);
    expect(isTagBoardDensity(2)).toBe(false);
  });
});

describe('DENSITY_MIN_COLUMN_PX', () => {
  it('makes extended exactly double, so the column count halves', () => {
    expect(DENSITY_MIN_COLUMN_PX.extended).toBe(DENSITY_MIN_COLUMN_PX.granular * 2);
  });

  it('halves the blocks per row at the same pane width', () => {
    // 1700px pane: 6 granular columns, 3 extended ones.
    expect(fitsInOneRow(6, 1700, DENSITY_MIN_COLUMN_PX.granular)).toBe(true);
    expect(fitsInOneRow(4, 1700, DENSITY_MIN_COLUMN_PX.extended)).toBe(false);
    expect(fitsInOneRow(3, 1700, DENSITY_MIN_COLUMN_PX.extended)).toBe(true);
  });
});

describe('planColumnLoads', () => {
  const board = (queryKey: string, ...values: string[]) =>
    tagBoardReducer(initialTagBoardState, {
      type: 'COLUMNS_LOADED',
      queryKey,
      stats: values.map((v) => stat(v, 1)),
    }).columns;

  const fresh = () => ({ queryKey: '', keys: new Set<string>() });

  it('queues every block on a first load', () => {
    const plan = planColumnLoads(board('q1', 'a', 'b'), fresh(), 'q1', false);
    expect(plan.toLoad.map((c) => c.value)).toEqual(['a', 'b']);
  });

  it('queues nothing on a second pass with the same ledger', () => {
    const columns = board('q1', 'a', 'b');
    const first = planColumnLoads(columns, fresh(), 'q1', false);
    const second = planColumnLoads(columns, first.ledger, 'q1', false);
    expect(second.toLoad).toEqual([]);
  });

  it('queues nothing while the block set is still being fetched', () => {
    // The columns on screen belong to the PREVIOUS window. Paging them now
    // wastes queries and — worse — marks their keys as requested, so the real
    // columns that replace them are never fetched. This is what left blocks
    // stuck on a skeleton after a range or search change.
    const plan = planColumnLoads(board('q1', 'a', 'b'), fresh(), 'q2', true);
    expect(plan.toLoad).toEqual([]);
  });

  it('leaves the ledger untouched while the block set is being fetched', () => {
    const ledger = fresh();
    const plan = planColumnLoads(board('q1', 'a'), ledger, 'q2', true);
    expect(plan.ledger.keys.size).toBe(0);
  });

  it('re-queues everything when the query changes', () => {
    // New window means the rows on screen were fetched under the old one.
    const first = planColumnLoads(board('q1', 'a', 'b'), fresh(), 'q1', false);
    const next = planColumnLoads(board('q2', 'a', 'b'), first.ledger, 'q2', false);
    expect(next.toLoad.map((c) => c.value)).toEqual(['a', 'b']);
  });

  it('queues only blocks that appeared since the last pass', () => {
    const first = planColumnLoads(board('q1', 'a'), fresh(), 'q1', false);
    const next = planColumnLoads(board('q1', 'a', 'b'), first.ledger, 'q1', false);
    expect(next.toLoad.map((c) => c.value)).toEqual(['b']);
  });

  it('carries the query key on the ledger it returns', () => {
    const plan = planColumnLoads(board('q1', 'a'), fresh(), 'q1', false);
    expect(plan.ledger.queryKey).toBe('q1');
  });
});

describe('normaliseCustomRange', () => {
  it('swaps a reversed pair so the boxes show the range actually applied', () => {
    // Silent normalisation inside rangeToWindow wasn't enough: the inputs kept
    // displaying the reversed pair, so the board looked like it was ignoring
    // the dates on screen. Correct the values themselves.
    expect(normaliseCustomRange({ from: '2026-09-10', to: '2026-06-01' })).toEqual({
      from: '2026-06-01',
      to: '2026-09-10',
    });
  });

  it('leaves an ordered pair alone', () => {
    const ordered = { from: '2026-06-01', to: '2026-09-10' };
    expect(normaliseCustomRange(ordered)).toEqual(ordered);
  });

  it('leaves a half-filled range alone so typing is not disrupted', () => {
    expect(normaliseCustomRange({ from: '2026-09-10', to: '' })).toEqual({ from: '2026-09-10', to: '' });
    expect(normaliseCustomRange({ from: '', to: '2026-06-01' })).toEqual({ from: '', to: '2026-06-01' });
  });

  it('leaves equal dates alone', () => {
    const same = { from: '2026-06-01', to: '2026-06-01' };
    expect(normaliseCustomRange(same)).toEqual(same);
  });

  it('leaves an unparseable value alone', () => {
    const partial = { from: '2026-06', to: '2026-09-10' };
    expect(normaliseCustomRange(partial)).toEqual(partial);
  });
});

describe('senderLabel', () => {
  const ME = 'me@mine.test';

  it('names the other person normally', () => {
    expect(senderLabel(email('e', { sender: 'Alice', senderEmail: 'alice@ex.test' }), ME, 'Me')).toBe('Alice');
  });

  it('says "Me" for a message the account owner sent', () => {
    // A card showing your own address tells you nothing you didn't know.
    expect(senderLabel(email('e', { sender: 'Gero Dp', senderEmail: ME }), ME, 'Me')).toBe('Me');
  });

  it('matches the owner address case-insensitively', () => {
    expect(senderLabel(email('e', { sender: 'Gero', senderEmail: 'Me@Mine.TEST' }), ME, 'Me')).toBe('Me');
  });

  it("trusts the provider's sent flag even when the address differs", () => {
    // Aliases and send-as addresses: still the user, still "Me".
    const alias = email('e', { sender: 'Gero', senderEmail: 'alias@mine.test', isSent: true });
    expect(senderLabel(alias, ME, 'Me')).toBe('Me');
  });

  it('falls back to the address when there is no display name', () => {
    expect(senderLabel(email('e', { sender: '', senderEmail: 'bare@ex.test' }), ME, 'Me')).toBe('bare@ex.test');
  });

  it('handles an unknown owner address without mislabelling anyone', () => {
    expect(senderLabel(email('e', { sender: 'Alice', senderEmail: 'alice@ex.test' }), '', 'Me')).toBe('Alice');
  });
});

describe('dedupeOrdinalThreads', () => {
  const withEmails = (queryKey: string, specs: [string, string[]][]) => {
    let state = tagBoardReducer(initialTagBoardState, {
      type: 'COLUMNS_LOADED',
      queryKey,
      stats: specs.map(([v]) => stat(v, 1)),
    });
    for (const [value, ids] of specs) {
      state = tagBoardReducer(state, {
        type: 'PAGE_LOADED',
        key: key(value),
        emails: ids.map((id) => email(id, { threadId: `thread-${id}` })),
        pageSize: TAG_BOARD_PAGE_SIZE,
      });
    }
    return state.columns;
  };

  it('keeps a thread only in the highest-ranked block that holds it', () => {
    // A thread whose messages carry two priorities matched both blocks — the
    // same conversation appeared under normal AND low. Blocks arrive ordered
    // urgent → normal → low, so the first one wins.
    const columns = withEmails('q', [
      ['normal', ['a', 'shared']],
      ['low', ['shared', 'b']],
    ]);
    const out = dedupeOrdinalThreads(columns, 'priority');
    expect(out[0].emails.map((e) => e.id)).toEqual(['a', 'shared']);
    expect(out[1].emails.map((e) => e.id)).toEqual(['b']);
  });

  it('leaves non-ordinal dimensions alone', () => {
    // A thread really can be about two topics, or involve two companies.
    const columns = withEmails('q', [
      ['billing', ['shared']],
      ['project', ['shared']],
    ]);
    const out = dedupeOrdinalThreads(columns, 'topic');
    expect(out[1].emails.map((e) => e.id)).toEqual(['shared']);
  });

  it('dedupes per account, not globally', () => {
    // Two accounts can carry the same provider thread id; they are different
    // conversations and both stay.
    let state = tagBoardReducer(initialTagBoardState, {
      type: 'COLUMNS_LOADED',
      queryKey: 'q',
      stats: [stat('normal', 1, 'acc-a'), stat('low', 1, 'acc-b')],
    });
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: columnKey('acc-a', 'normal'),
      emails: [email('x', { accountId: 'acc-a', threadId: 't' })],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });
    state = tagBoardReducer(state, {
      type: 'PAGE_LOADED',
      key: columnKey('acc-b', 'low'),
      emails: [email('y', { accountId: 'acc-b', threadId: 't' })],
      pageSize: TAG_BOARD_PAGE_SIZE,
    });

    const out = dedupeOrdinalThreads(state.columns, 'priority');
    expect(out[1].emails).toHaveLength(1);
  });

  it('returns the same column objects when nothing is duplicated', () => {
    const columns = withEmails('q', [
      ['normal', ['a']],
      ['low', ['b']],
    ]);
    expect(dedupeOrdinalThreads(columns, 'priority')).toEqual(columns);
  });

  it('does not disturb a block that is still empty', () => {
    const columns = withEmails('q', [['normal', []]]);
    expect(dedupeOrdinalThreads(columns, 'priority')[0].emails).toEqual([]);
  });
});
