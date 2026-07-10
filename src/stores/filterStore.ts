import { create } from 'zustand';
import * as api from '@/lib/api';
import { toQueryAccountId } from '@/stores/accountStore';
import type { ActiveFilter, SmartFilter, SmartFilterPref, TagPriority } from '@/types';

const COMPANY_PRIORITY_LIMIT = 15;

function applyCompanyPriority(suggestions: SmartFilter[], priorities: TagPriority[]): SmartFilter[] {
  // Build lookup for existing company counts so we preserve the displayed email count.
  const countByValue = new Map<string, number>();
  for (const s of suggestions) {
    if (s.type === 'company') countByValue.set(s.value, s.count);
  }

  // Keep every non-company suggestion in original order.
  const nonCompany = suggestions.filter((s) => s.type !== 'company');

  // Priority-ordered company list. Fall back to sent+received when the tag isn't
  // yet represented in smart_filter_suggestions (e.g. during first backfill).
  const companyFilters: SmartFilter[] = priorities.map((p) => ({
    type: 'company',
    value: p.tagValue,
    count: countByValue.get(p.tagValue) ?? p.sentCount + p.receivedCount,
  }));

  return [...nonCompany, ...companyFilters];
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
    const [saved, priorities] = await Promise.all([
      api.getSavedSuggestions(queryId),
      api.getTagPriorities(queryId, 'company', COMPANY_PRIORITY_LIMIT).catch(() => [] as TagPriority[]),
    ]);
    // Discard result if account switched again while the request was in flight.
    if (get().currentAccountId !== accountId) return;
    dispatch(set, {
      type: 'SET_SUGGESTIONS',
      suggestions: applyCompanyPriority(suggestionsToSmartFilters(saved), priorities),
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
      const [saved, prefs, priorities] = await Promise.all([
        api.getSavedSuggestions(queryId),
        api.getFilterPrefs(queryId),
        api.getTagPriorities(queryId, 'company', COMPANY_PRIORITY_LIMIT).catch(() => [] as TagPriority[]),
      ]);

      // Discard if account switched while stats were being computed.
      if (get().currentAccountId !== accountId) return;
      dispatch(set, {
        type: 'SET_SUGGESTIONS',
        suggestions: applyCompanyPriority(suggestionsToSmartFilters(saved), priorities),
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
