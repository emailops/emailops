import { create } from 'zustand';
import * as api from '@/lib/api';
import { toQueryAccountId } from '@/stores/accountStore';
import type { ActiveFilter, SmartFilter, SmartFilterPref, TagStat } from '@/types';

const TAG_RANK_LIMIT = 40;

/** Tag types the classifier emits, in the order the sidebar groups them.
 *  Mirrors the Rust `CLASSIFIED_TAG_TYPES` and `SmartFilters.tsx`. */
const CLASSIFIED_TAG_TYPES = ['company', 'priority', 'intent', 'topic'] as const;

/** Ranked board rows per tag type. */
export type RankedTagStats = Partial<Record<(typeof CLASSIFIED_TAG_TYPES)[number], TagStat[]>>;

/**
 * Build the sidebar's classified-tag filters straight from the tag board's
 * ranking, so the two surfaces show the same tags in the same order.
 *
 * They used to disagree on the *set*, not just the order: the sidebar listed
 * whatever `refresh_filter_stats` had cached — the top values by raw count —
 * and then re-sorted that. Anything the board surfaced on engagement but that
 * missed the count cut simply wasn't there to reorder.
 *
 * The board ranks `(account, tag)` pairs; the sidebar filters the whole scope,
 * so a value appears once, at its best rank, with the per-account counts summed
 * (each counts distinct threads within its account, so the sum is the total).
 */
export function buildTagFilters(ranked: RankedTagStats): SmartFilter[] {
  const filters: SmartFilter[] = [];

  for (const type of CLASSIFIED_TAG_TYPES) {
    const stats = ranked[type];
    if (!stats || stats.length === 0) continue;

    const order: string[] = [];
    const totals = new Map<string, number>();
    for (const s of stats) {
      if (!totals.has(s.tagValue)) order.push(s.tagValue);
      totals.set(s.tagValue, (totals.get(s.tagValue) ?? 0) + s.count);
    }
    for (const value of order) {
      filters.push({ type, value, count: totals.get(value) ?? 0 });
    }
  }

  return filters;
}

/**
 * Ask the board's ranking for each classified tag type. A failed type
 * contributes nothing rather than rejecting the batch — the sidebar loses that
 * one group, not the whole section.
 */
async function fetchRankedTags(queryId: string | null): Promise<RankedTagStats> {
  const results = await Promise.all(
    CLASSIFIED_TAG_TYPES.map((type) =>
      api
        .getTagBoardStats(queryId, type, undefined, TAG_RANK_LIMIT)
        .then((stats) => [type, stats] as const)
        // A failed type contributes nothing rather than rejecting the batch —
        // the sidebar loses that group, not the whole section.
        .catch(() => [type, [] as TagStat[]] as const),
    ),
  );

  const ranked: RankedTagStats = {};
  for (const [type, stats] of results) {
    if (stats.length > 0) ranked[type] = stats;
  }
  return ranked;
}

/** Contacts and domains from the saved cache, then classified tags from the
 *  board's live ranking. */
function mergeSuggestions(saved: SmartFilter[], ranked: RankedTagStats): SmartFilter[] {
  const tagTypes = new Set<string>(CLASSIFIED_TAG_TYPES);
  return [...saved.filter((s) => !tagTypes.has(s.type)), ...buildTagFilters(ranked)];
}

function suggestionsToSmartFilters(raw: { filterType: string; filterValue: string; count: number }[]): SmartFilter[] {
  return raw.map((s) => ({
    type: s.filterType as SmartFilter['type'],
    value: s.filterValue,
    count: s.count,
  }));
}

// ── Pure state ────────────────────────────────────────────────────────────────

export interface FilterState {
  suggestions: SmartFilter[];
  prefs: SmartFilterPref[];
  activeFilter: ActiveFilter | null;
  currentAccountId: string | null;
  isLoadingStats: boolean;
}

export const initialFilterState: FilterState = {
  suggestions: [],
  prefs: [],
  activeFilter: null,
  currentAccountId: null,
  isLoadingStats: false,
};

export type FilterAction =
  | { type: 'SET_ACCOUNT_ID'; accountId: string }
  | { type: 'SET_SUGGESTIONS'; suggestions: SmartFilter[] }
  | { type: 'SET_PREFS'; prefs: SmartFilterPref[] }
  | { type: 'SET_LOADING_STATS'; loading: boolean }
  | { type: 'TOGGLE_FILTER'; filter: ActiveFilter }
  | { type: 'CLEAR_ACTIVE_FILTER' }
  | { type: 'RESET' };

export function filterReducer(state: FilterState, action: FilterAction): FilterState {
  switch (action.type) {
    case 'SET_ACCOUNT_ID':
      return { ...state, currentAccountId: action.accountId };
    case 'SET_SUGGESTIONS':
      return { ...state, suggestions: action.suggestions };
    case 'SET_PREFS':
      return { ...state, prefs: action.prefs };
    case 'SET_LOADING_STATS':
      return { ...state, isLoadingStats: action.loading };
    case 'TOGGLE_FILTER': {
      const isSame =
        state.activeFilter?.type === action.filter.type && state.activeFilter?.value === action.filter.value;
      return { ...state, activeFilter: isSame ? null : action.filter };
    }
    case 'CLEAR_ACTIVE_FILTER':
      return { ...state, activeFilter: null };
    case 'RESET':
      return initialFilterState;
    default:
      return state;
  }
}

// ── Selectors ─────────────────────────────────────────────────────────────────

export function selectActiveFilter(state: FilterState): ActiveFilter | null {
  return state.activeFilter;
}

export function selectIsLoadingStats(state: FilterState): boolean {
  return state.isLoadingStats;
}

// Sender addresses are case-insensitive identifiers (a pref saved from an
// email header may differ in case from the stored suggestion); other filter
// values are display strings and stay case-sensitive.
export function filterMatchKey(type: string, value: string): string {
  return type === 'sender' ? `${type}:${value.toLowerCase()}` : `${type}:${value}`;
}

export function selectDisplayedFilters(state: FilterState): SmartFilter[] {
  const { suggestions, prefs } = state;

  const pinnedSet = new Set<string>();
  const removedSet = new Set<string>();
  const pinnedFilters: SmartFilter[] = [];

  for (const pref of prefs) {
    const key = filterMatchKey(pref.filterType, pref.filterValue);
    if (pref.status === 'pinned') {
      pinnedSet.add(key);
      const suggestion = suggestions.find((s) => filterMatchKey(s.type, s.value) === key);
      pinnedFilters.push({
        type: pref.filterType as SmartFilter['type'],
        value: pref.filterValue,
        count: suggestion?.count ?? 0,
      });
    } else if (pref.status === 'removed') {
      removedSet.add(key);
    }
  }

  const suggestedFilters = suggestions.filter((s) => {
    const key = filterMatchKey(s.type, s.value);
    return !pinnedSet.has(key) && !removedSet.has(key);
  });

  return [...pinnedFilters, ...suggestedFilters];
}

// ── Zustand store ─────────────────────────────────────────────────────────────

interface FilterStore extends FilterState {
  loadSaved: (accountId: string) => Promise<void>;
  fetchPrefs: (accountId: string) => Promise<void>;
  forceRefresh: (accountId: string) => Promise<void>;
  toggleFilter: (filter: ActiveFilter) => void;
  clearActiveFilter: () => void;
  pinFilter: (accountId: string, filter: ActiveFilter) => Promise<void>;
  unpinFilter: (accountId: string, filter: ActiveFilter) => Promise<void>;
  removeFilter: (accountId: string, filter: ActiveFilter) => Promise<void>;
  restoreFilter: (accountId: string, filter: ActiveFilter) => Promise<void>;
  addSenderAsFilter: (accountId: string, senderEmail: string) => Promise<void>;
  getDisplayedFilters: () => SmartFilter[];
  reset: () => void;
}

function dispatch(set: (fn: (s: FilterState) => FilterState) => void, action: FilterAction): void {
  set((s) => filterReducer(s, action));
}

export const useFilterStore = create<FilterStore>((set, get) => ({
  ...initialFilterState,

  // The `accountId` params below keep the UI identity (which may be the
  // All-accounts sentinel) for stale-response tracking; every api.* call
  // translates via toQueryAccountId (sentinel → null = all enabled accounts).
  loadSaved: async (accountId) => {
    // Clear stale suggestions immediately so old account's filters aren't visible
    // while loading. Also set currentAccountId now so concurrent calls for a
    // previous account can detect they've been superseded.
    dispatch(set, { type: 'SET_ACCOUNT_ID', accountId });
    dispatch(set, { type: 'SET_SUGGESTIONS', suggestions: [] });
    const queryId = toQueryAccountId(accountId);
    const [saved, ranked] = await Promise.all([api.getSavedSuggestions(queryId), fetchRankedTags(queryId)]);
    // Discard result if account switched again while the request was in flight.
    if (get().currentAccountId !== accountId) return;
    dispatch(set, {
      type: 'SET_SUGGESTIONS',
      suggestions: mergeSuggestions(suggestionsToSmartFilters(saved), ranked),
    });
  },

  fetchPrefs: async (accountId) => {
    const prefs = await api.getFilterPrefs(toQueryAccountId(accountId));
    // Discard if the account switched while the request was in flight.
    if (get().currentAccountId !== accountId) return;
    dispatch(set, { type: 'SET_PREFS', prefs });
  },

  forceRefresh: async (accountId) => {
    dispatch(set, { type: 'SET_ACCOUNT_ID', accountId });
    dispatch(set, { type: 'SET_LOADING_STATS', loading: true });
    const queryId = toQueryAccountId(accountId);
    try {
      // Refresh computes stats and saves all suggestions (domains, senders, tags) to DB
      await api.refreshFilterStats(queryId);

      // Reload from DB to get the full set including tag-based suggestions,
      // and fetch priority ordering for companies in parallel.
      const [saved, prefs, ranked] = await Promise.all([
        api.getSavedSuggestions(queryId),
        api.getFilterPrefs(queryId),
        fetchRankedTags(queryId),
      ]);

      // Discard if account switched while stats were being computed.
      if (get().currentAccountId !== accountId) return;
      dispatch(set, {
        type: 'SET_SUGGESTIONS',
        suggestions: mergeSuggestions(suggestionsToSmartFilters(saved), ranked),
      });
      dispatch(set, { type: 'SET_PREFS', prefs });
    } finally {
      // Always release the spinner — including the account-switched early
      // return above, which used to leave it stuck on forever.
      dispatch(set, { type: 'SET_LOADING_STATS', loading: false });
    }
  },

  toggleFilter: (filter) => dispatch(set, { type: 'TOGGLE_FILTER', filter }),

  clearActiveFilter: () => dispatch(set, { type: 'CLEAR_ACTIVE_FILTER' }),

  pinFilter: async (accountId, filter) => {
    const queryId = toQueryAccountId(accountId);
    await api.pinFilter(queryId, filter.type, filter.value);
    const prefs = await api.getFilterPrefs(queryId);
    dispatch(set, { type: 'SET_PREFS', prefs });
  },

  unpinFilter: async (accountId, filter) => {
    const queryId = toQueryAccountId(accountId);
    await api.deleteFilterPref(queryId, filter.type, filter.value);
    const prefs = await api.getFilterPrefs(queryId);
    dispatch(set, { type: 'SET_PREFS', prefs });
  },

  removeFilter: async (accountId, filter) => {
    const queryId = toQueryAccountId(accountId);
    await api.removeFilter(queryId, filter.type, filter.value);
    const prefs = await api.getFilterPrefs(queryId);
    dispatch(set, { type: 'SET_PREFS', prefs });
    const { activeFilter } = get();
    if (activeFilter?.type === filter.type && activeFilter?.value === filter.value) {
      dispatch(set, { type: 'CLEAR_ACTIVE_FILTER' });
    }
  },

  addSenderAsFilter: async (accountId, senderEmail) => {
    const queryId = toQueryAccountId(accountId);
    await api.pinFilter(queryId, 'sender', senderEmail);
    const prefs = await api.getFilterPrefs(queryId);
    dispatch(set, { type: 'SET_PREFS', prefs });
  },

  restoreFilter: async (accountId, filter) => {
    const queryId = toQueryAccountId(accountId);
    await api.deleteFilterPref(queryId, filter.type, filter.value);
    const prefs = await api.getFilterPrefs(queryId);
    dispatch(set, { type: 'SET_PREFS', prefs });
  },

  getDisplayedFilters: () => selectDisplayedFilters(get()),

  reset: () => dispatch(set, { type: 'RESET' }),
}));
