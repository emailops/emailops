// Unit tests for filterStore: pure reducer + selectors (plain function calls)
// and async store actions (Zustand store with the api layer mocked).

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ActiveFilter, SmartFilter, SmartFilterPref } from '@/types';
import {
  type FilterAction,
  type FilterState,
  filterReducer,
  initialFilterState,
  selectActiveFilter,
  selectDisplayedFilters,
  selectIsLoadingStats,
  useFilterStore,
} from './filterStore';

vi.mock('@/lib/api', () => ({
  getSavedSuggestions: vi.fn(async () => []),
  getTagPriorities: vi.fn(async () => []),
  getFilterPrefs: vi.fn(async () => []),
  refreshFilterStats: vi.fn(async () => ({ topDomains: [], topSenders: [] })),
  pinFilter: vi.fn(async () => {}),
  removeFilter: vi.fn(async () => {}),
  deleteFilterPref: vi.fn(async () => {}),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';

// ── helpers ───────────────────────────────────────────────────────────────────

function makeSuggestion(type: SmartFilter['type'], value: string, count = 5): SmartFilter {
  return { type, value, count };
}

function makePref(filterType: string, filterValue: string, status: 'pinned' | 'removed'): SmartFilterPref {
  return { id: `${filterType}:${filterValue}`, filterType, filterValue, status, accountId: 'acc-1' };
}

function makeFilter(type: ActiveFilter['type'], value: string): ActiveFilter {
  return { type, value };
}

function reduce(state: FilterState, action: FilterAction): FilterState {
  return filterReducer(state, action);
}

// ── SET_ACCOUNT_ID ────────────────────────────────────────────────────────────

describe('SET_ACCOUNT_ID', () => {
  it('updates currentAccountId', () => {
    const s = reduce(initialFilterState, { type: 'SET_ACCOUNT_ID', accountId: 'acc-42' });
    expect(s.currentAccountId).toBe('acc-42');
  });

  it('does not touch other fields', () => {
    const s0: FilterState = { ...initialFilterState, isLoadingStats: true };
    const s = reduce(s0, { type: 'SET_ACCOUNT_ID', accountId: 'x' });
    expect(s.isLoadingStats).toBe(true);
  });
});

// ── SET_SUGGESTIONS ───────────────────────────────────────────────────────────

describe('SET_SUGGESTIONS', () => {
  it('replaces suggestions list', () => {
    const suggestions = [makeSuggestion('domain', 'acme.com')];
    const s = reduce(initialFilterState, { type: 'SET_SUGGESTIONS', suggestions });
    expect(s.suggestions).toEqual(suggestions);
  });

  it('clears to empty when given []', () => {
    const s0: FilterState = {
      ...initialFilterState,
      suggestions: [makeSuggestion('sender', 'a@b.com')],
    };
    const s = reduce(s0, { type: 'SET_SUGGESTIONS', suggestions: [] });
    expect(s.suggestions).toHaveLength(0);
  });
});

// ── SET_PREFS ─────────────────────────────────────────────────────────────────

describe('SET_PREFS', () => {
  it('replaces prefs list', () => {
    const prefs = [makePref('domain', 'acme.com', 'pinned')];
    const s = reduce(initialFilterState, { type: 'SET_PREFS', prefs });
    expect(s.prefs).toEqual(prefs);
  });
});

// ── SET_LOADING_STATS ─────────────────────────────────────────────────────────

describe('SET_LOADING_STATS', () => {
  it('sets isLoadingStats to true', () => {
    const s = reduce(initialFilterState, { type: 'SET_LOADING_STATS', loading: true });
    expect(s.isLoadingStats).toBe(true);
  });

  it('clears isLoadingStats', () => {
    const s0: FilterState = { ...initialFilterState, isLoadingStats: true };
    const s = reduce(s0, { type: 'SET_LOADING_STATS', loading: false });
    expect(s.isLoadingStats).toBe(false);
  });
});

// ── TOGGLE_FILTER ─────────────────────────────────────────────────────────────

describe('TOGGLE_FILTER', () => {
  it('sets activeFilter when none is active', () => {
    const filter = makeFilter('domain', 'acme.com');
    const s = reduce(initialFilterState, { type: 'TOGGLE_FILTER', filter });
    expect(s.activeFilter).toEqual(filter);
  });

  it('clears activeFilter when same filter is toggled again', () => {
    const filter = makeFilter('sender', 'bob@acme.com');
    const s0: FilterState = { ...initialFilterState, activeFilter: filter };
    const s = reduce(s0, { type: 'TOGGLE_FILTER', filter });
    expect(s.activeFilter).toBeNull();
  });

  it('replaces activeFilter when a different filter is toggled', () => {
    const old = makeFilter('domain', 'acme.com');
    const next = makeFilter('sender', 'alice@acme.com');
    const s0: FilterState = { ...initialFilterState, activeFilter: old };
    const s = reduce(s0, { type: 'TOGGLE_FILTER', filter: next });
    expect(s.activeFilter).toEqual(next);
  });

  it('uses type+value equality (same type, different value → replace)', () => {
    const s0: FilterState = {
      ...initialFilterState,
      activeFilter: makeFilter('domain', 'acme.com'),
    };
    const s = reduce(s0, { type: 'TOGGLE_FILTER', filter: makeFilter('domain', 'other.com') });
    expect(s.activeFilter?.value).toBe('other.com');
  });
});

// ── CLEAR_ACTIVE_FILTER ───────────────────────────────────────────────────────

describe('CLEAR_ACTIVE_FILTER', () => {
  it('sets activeFilter to null', () => {
    const s0: FilterState = {
      ...initialFilterState,
      activeFilter: makeFilter('domain', 'acme.com'),
    };
    const s = reduce(s0, { type: 'CLEAR_ACTIVE_FILTER' });
    expect(s.activeFilter).toBeNull();
  });

  it('is a no-op when already null', () => {
    const s = reduce(initialFilterState, { type: 'CLEAR_ACTIVE_FILTER' });
    expect(s.activeFilter).toBeNull();
  });
});

// ── RESET ─────────────────────────────────────────────────────────────────────

describe('RESET', () => {
  it('restores initial state', () => {
    const s0: FilterState = {
      suggestions: [makeSuggestion('domain', 'x.com')],
      prefs: [makePref('domain', 'x.com', 'pinned')],
      activeFilter: makeFilter('domain', 'x.com'),
      currentAccountId: 'acc-99',
      isLoadingStats: true,
    };
    const s = reduce(s0, { type: 'RESET' });
    expect(s).toEqual(initialFilterState);
  });
});

// ── Selectors ──────────────────────────────────────────────────────────────────

describe('selectActiveFilter', () => {
  it('returns null when nothing active', () => {
    expect(selectActiveFilter(initialFilterState)).toBeNull();
  });

  it('returns the active filter', () => {
    const filter = makeFilter('domain', 'acme.com');
    const s: FilterState = { ...initialFilterState, activeFilter: filter };
    expect(selectActiveFilter(s)).toEqual(filter);
  });
});

describe('selectIsLoadingStats', () => {
  it('false initially', () => {
    expect(selectIsLoadingStats(initialFilterState)).toBe(false);
  });

  it('true when set', () => {
    const s: FilterState = { ...initialFilterState, isLoadingStats: true };
    expect(selectIsLoadingStats(s)).toBe(true);
  });
});

describe('selectDisplayedFilters', () => {
  it('returns all suggestions when no prefs', () => {
    const suggestions = [makeSuggestion('domain', 'a.com'), makeSuggestion('sender', 'b@c.com')];
    const s: FilterState = { ...initialFilterState, suggestions };
    expect(selectDisplayedFilters(s)).toEqual(suggestions);
  });

  it('excludes removed suggestions', () => {
    const suggestions = [makeSuggestion('domain', 'a.com'), makeSuggestion('domain', 'b.com')];
    const prefs = [makePref('domain', 'a.com', 'removed')];
    const s: FilterState = { ...initialFilterState, suggestions, prefs };
    const displayed = selectDisplayedFilters(s);
    expect(displayed.map((f) => f.value)).toEqual(['b.com']);
  });

  it('puts pinned filters first', () => {
    const suggestions = [makeSuggestion('domain', 'a.com'), makeSuggestion('domain', 'b.com')];
    const prefs = [makePref('domain', 'b.com', 'pinned')];
    const s: FilterState = { ...initialFilterState, suggestions, prefs };
    const displayed = selectDisplayedFilters(s);
    expect(displayed[0].value).toBe('b.com');
    expect(displayed[1].value).toBe('a.com');
  });

  it('pinned filter not present in suggestions gets count 0', () => {
    const prefs = [makePref('domain', 'new.com', 'pinned')];
    const s: FilterState = { ...initialFilterState, suggestions: [], prefs };
    const displayed = selectDisplayedFilters(s);
    expect(displayed).toHaveLength(1);
    expect(displayed[0].value).toBe('new.com');
    expect(displayed[0].count).toBe(0);
  });

  it('empty state returns empty list', () => {
    expect(selectDisplayedFilters(initialFilterState)).toHaveLength(0);
  });

  // Sender addresses are case-insensitive identifiers: a pref saved with one
  // casing (e.g. blockSender with the header's casing) must match a suggestion
  // stored with another.
  it('hides a sender suggestion removed under a different case', () => {
    const suggestions = [makeSuggestion('sender', 'alice@ex.com')];
    const prefs = [makePref('sender', 'Alice@Ex.com', 'removed')];
    const s: FilterState = { ...initialFilterState, suggestions, prefs };
    expect(selectDisplayedFilters(s)).toHaveLength(0);
  });

  it('does not duplicate a sender pinned under a different case', () => {
    const suggestions = [makeSuggestion('sender', 'alice@ex.com', 7)];
    const prefs = [makePref('sender', 'Alice@Ex.com', 'pinned')];
    const s: FilterState = { ...initialFilterState, suggestions, prefs };
    const displayed = selectDisplayedFilters(s);
    expect(displayed).toHaveLength(1);
    expect(displayed[0].count).toBe(7);
  });

  it('keeps non-sender filter values case-sensitive', () => {
    // Tag values like company names are display strings, not addresses —
    // "Acme" and "acme" stay distinct.
    const suggestions = [makeSuggestion('company', 'acme')];
    const prefs = [makePref('company', 'Acme', 'removed')];
    const s: FilterState = { ...initialFilterState, suggestions, prefs };
    expect(selectDisplayedFilters(s).map((f) => f.value)).toEqual(['acme']);
  });
});

// ── Async store actions (mocked api) ─────────────────────────────────────────

describe('forceRefresh', () => {
  beforeEach(() => {
    useFilterStore.setState({ ...initialFilterState });
    vi.clearAllMocks();
  });

  it('clears isLoadingStats when the account switches mid-refresh', async () => {
    vi.mocked(api.refreshFilterStats).mockImplementationOnce(async () => {
      // Simulate the user switching accounts while stats compute.
      useFilterStore.setState({ currentAccountId: 'acc-B' });
      return { topDomains: [], topSenders: [] };
    });

    await useFilterStore.getState().forceRefresh('acc-A');

    expect(useFilterStore.getState().isLoadingStats).toBe(false);
  });
});

describe('fetchPrefs', () => {
  beforeEach(() => {
    useFilterStore.setState({ ...initialFilterState });
    vi.clearAllMocks();
  });

  it('discards a stale response after the account switched', async () => {
    useFilterStore.setState({ currentAccountId: 'acc-A' });
    vi.mocked(api.getFilterPrefs).mockImplementationOnce(async () => {
      useFilterStore.setState({ currentAccountId: 'acc-B' });
      return [makePref('domain', 'stale.com', 'pinned')];
    });

    await useFilterStore.getState().fetchPrefs('acc-A');

    expect(useFilterStore.getState().prefs).toHaveLength(0);
  });

  it('applies the response when the account is unchanged', async () => {
    useFilterStore.setState({ currentAccountId: 'acc-A' });
    vi.mocked(api.getFilterPrefs).mockResolvedValueOnce([makePref('domain', 'fresh.com', 'pinned')]);

    await useFilterStore.getState().fetchPrefs('acc-A');

    expect(useFilterStore.getState().prefs.map((p) => p.filterValue)).toEqual(['fresh.com']);
  });
});

describe('unified (All accounts) mode', () => {
  beforeEach(() => {
    useFilterStore.setState({ ...initialFilterState });
    vi.clearAllMocks();
  });

  it('loadSaved with the sentinel id queries the API with accountId: null', async () => {
    const { ALL_ACCOUNTS_ID } = await import('./accountStore');
    await useFilterStore.getState().loadSaved(ALL_ACCOUNTS_ID);

    expect(vi.mocked(api.getSavedSuggestions)).toHaveBeenCalledWith(null);
    expect(vi.mocked(api.getTagPriorities)).toHaveBeenCalledWith(null, 'company', expect.any(Number));
    // Identity tracking keeps the sentinel, so stale-response guards still work.
    expect(useFilterStore.getState().currentAccountId).toBe(ALL_ACCOUNTS_ID);
  });

  it('pinFilter with the sentinel id writes and re-reads with accountId: null', async () => {
    const { ALL_ACCOUNTS_ID } = await import('./accountStore');
    await useFilterStore.getState().pinFilter(ALL_ACCOUNTS_ID, makeFilter('domain', 'acme.com'));

    expect(vi.mocked(api.pinFilter)).toHaveBeenCalledWith(null, 'domain', 'acme.com');
    expect(vi.mocked(api.getFilterPrefs)).toHaveBeenCalledWith(null);
  });

  it('a real account id passes through unchanged', async () => {
    await useFilterStore.getState().fetchPrefs('acc-1');
    expect(vi.mocked(api.getFilterPrefs)).toHaveBeenCalledWith('acc-1');
  });
});
