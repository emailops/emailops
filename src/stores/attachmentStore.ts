import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Attachment, AttachmentRule, AttachmentRuleSuggestion } from '@/types';

const PAGE_SIZE = 50;

/** Progress of applying one rule to existing mail. */
export interface RuleApplyState {
  processed: number;
  total: number;
  /** New attachments collected by this run. */
  saved: number;
  /** Every attachment the rule holds once the run is done (new + earlier). */
  collected?: number;
  status: 'running' | 'done' | 'failed';
  /** Which run this is: a newer apply of the rule supersedes an older one. */
  runId: string;
}

export interface RuleApplyProgress {
  ruleId: string;
  runId: string;
  processed: number;
  total: number;
  saved: number;
}

interface AttachmentStore {
  // Rules
  rules: AttachmentRule[];
  isLoadingRules: boolean;

  // Candidate rules mined from recurring document attachments, for the
  // account in `suggestionsAccountId`.
  suggestions: AttachmentRuleSuggestion[];
  suggestionsAccountId: string | null;
  /** A re-mine is running (the modal says it is looking). */
  suggestionsLoading: boolean;
  /** Dismissed suggestions of that account, most recent first, for undo. */
  dismissedSuggestions: AttachmentRuleSuggestion[];

  // Rule applies to existing mail, per rule id. Kept here, not in the rules
  // modal, so a scan still shows its progress after the modal is reopened.
  ruleApplies: Record<string, RuleApplyState>;

  // Attachments list
  attachments: Attachment[];
  selectedAttachment: Attachment | null;
  checkedIds: Set<string>;
  isLoading: boolean;
  isLoadingMore: boolean;
  hasMore: boolean;
  totalCount: number;

  // Filters
  selectedTag: string | null;
  availableTags: string[];

  // Error
  error: string | null;

  // Race condition prevention
  currentFetchId: number;

  // Rule actions
  fetchRules: (accountId: string) => Promise<void>;
  createRule: (
    accountId: string,
    name: string,
    senderEmailPattern: string | null,
    subjectPattern: string | null,
    filenamePattern: string | null,
    tags: string[],
  ) => Promise<AttachmentRule>;
  updateRule: (
    accountId: string,
    ruleId: string,
    name: string,
    senderEmailPattern: string | null,
    subjectPattern: string | null,
    filenamePattern: string | null,
    tags: string[],
    enabled: boolean,
  ) => Promise<AttachmentRule>;
  deleteRule: (ruleId: string, accountId: string) => Promise<void>;

  // Suggestion actions
  fetchSuggestions: (accountId: string) => Promise<void>;
  refreshSuggestions: (accountId: string) => Promise<void>;
  dismissSuggestion: (accountId: string, suggestionId: string) => Promise<void>;
  acceptSuggestion: (accountId: string, suggestionId: string) => Promise<void>;
  fetchDismissedSuggestions: (accountId: string) => Promise<void>;
  restoreSuggestion: (accountId: string, suggestionId: string) => Promise<void>;

  // Rule apply actions. `beginRuleApply` returns the run id the other
  // actions take, so a superseded run can never overwrite the newer one.
  beginRuleApply: (ruleId: string) => string;
  reportRuleApplyProgress: (progress: RuleApplyProgress) => void;
  finishRuleApply: (ruleId: string, runId: string, saved: number, collected: number | undefined) => void;
  failRuleApply: (ruleId: string, runId: string) => void;
  dropRuleApply: (ruleId: string, runId: string) => void;

  // Attachment actions
  fetchAttachments: (accountId: string, tag?: string | null) => Promise<void>;
  loadMoreAttachments: (accountId: string) => Promise<void>;
  selectAttachment: (attachment: Attachment | null) => void;
  toggleChecked: (id: string) => void;
  toggleCheckAll: () => void;
  clearChecked: () => void;

  // Tag actions
  setSelectedTag: (tag: string | null) => void;
  fetchTags: (accountId: string) => Promise<void>;

  // Utility
  clearError: () => void;
  reset: () => void;
}

/** Pending suggestion count, shown as a badge next to the rules entry points. */
export const selectSuggestionCount = (state: Pick<AttachmentStore, 'suggestions'>): number => state.suggestions.length;

const withoutSuggestion = (suggestions: AttachmentRuleSuggestion[], id: string) =>
  suggestions.filter((s) => s.id !== id);

// Monotonic id of the latest suggestion load, so a slow response for an
// account the user already switched away from never overwrites the list.
let suggestionsLoadId = 0;

// Suggestions the user accepted or dismissed in this session. A load that was
// already in flight when they did must not bring them back.
const resolvedSuggestionIds = new Set<string>();

let ruleApplyRunCounter = 0;

async function loadSuggestions(
  set: (partial: Partial<AttachmentStore>) => void,
  get: () => AttachmentStore,
  accountId: string,
  load: (accountId: string) => Promise<AttachmentRuleSuggestion[]>,
) {
  const loadId = ++suggestionsLoadId;
  // Another account's suggestions must not linger (badge, Review) while
  // this one loads — or after its load fails.
  if (get().suggestionsAccountId !== accountId) {
    set({ suggestions: [], dismissedSuggestions: [], suggestionsAccountId: accountId });
  }
  set({ suggestionsLoading: true });
  try {
    const suggestions = await load(accountId);
    if (loadId === suggestionsLoadId) {
      set({
        suggestions: suggestions.filter((s) => !resolvedSuggestionIds.has(s.id)),
        suggestionsAccountId: accountId,
      });
    }
  } catch (error) {
    if (loadId === suggestionsLoadId) set({ error: errorText(error) });
    throw error;
  } finally {
    if (loadId === suggestionsLoadId) set({ suggestionsLoading: false });
  }
}

async function resolveSuggestion(
  set: (partial: Partial<AttachmentStore> | ((state: AttachmentStore) => Partial<AttachmentStore>)) => void,
  suggestionId: string,
  resolve: () => Promise<void>,
) {
  try {
    await resolve();
  } catch (error) {
    set({ error: errorText(error) });
    throw error;
  }
  resolvedSuggestionIds.add(suggestionId);
  set((state) => ({ suggestions: withoutSuggestion(state.suggestions, suggestionId) }));
}

/** Apply `update` to the rule's apply state only while `runId` is its current run. */
function forRun(
  applies: Record<string, RuleApplyState>,
  ruleId: string,
  runId: string,
  update: (current: RuleApplyState) => RuleApplyState | null,
): Record<string, RuleApplyState> {
  const current = applies[ruleId];
  if (current?.runId !== runId) return applies;
  const next = update(current);
  if (next) return { ...applies, [ruleId]: next };
  const { [ruleId]: _dropped, ...rest } = applies;
  return rest;
}

export const useAttachmentStore = create<AttachmentStore>((set, get) => ({
  rules: [],
  isLoadingRules: false,
  suggestions: [],
  suggestionsAccountId: null,
  suggestionsLoading: false,
  dismissedSuggestions: [],
  ruleApplies: {},
  attachments: [],
  selectedAttachment: null,
  checkedIds: new Set<string>(),
  isLoading: false,
  isLoadingMore: false,
  hasMore: false,
  totalCount: 0,
  selectedTag: null,
  availableTags: [],
  error: null,
  currentFetchId: 0,

  fetchRules: async (accountId) => {
    set({ isLoadingRules: true });
    try {
      const rules = await api.listAttachmentRules(accountId);
      set({ rules, isLoadingRules: false });
    } catch (error) {
      set({ error: errorText(error), isLoadingRules: false });
    }
  },

  createRule: async (accountId, name, senderEmailPattern, subjectPattern, filenamePattern, tags) => {
    const rule = await api.createAttachmentRule(
      accountId,
      name,
      senderEmailPattern,
      subjectPattern,
      filenamePattern,
      tags,
    );
    set((state) => ({ rules: [rule, ...state.rules] }));
    return rule;
  },

  updateRule: async (accountId, ruleId, name, senderEmailPattern, subjectPattern, filenamePattern, tags, enabled) => {
    const rule = await api.updateAttachmentRule(
      accountId,
      ruleId,
      name,
      senderEmailPattern,
      subjectPattern,
      filenamePattern,
      tags,
      enabled,
    );
    set((state) => ({
      rules: state.rules.map((r) => (r.id === ruleId ? rule : r)),
    }));
    return rule;
  },

  deleteRule: async (ruleId, accountId) => {
    await api.deleteAttachmentRule(ruleId, accountId);
    set((state) => ({
      rules: state.rules.filter((r) => r.id !== ruleId),
    }));
  },

  fetchSuggestions: (accountId) => loadSuggestions(set, get, accountId, api.listAttachmentRuleSuggestions),

  refreshSuggestions: (accountId) => loadSuggestions(set, get, accountId, api.refreshAttachmentRuleSuggestions),

  dismissSuggestion: async (accountId, suggestionId) => {
    const dismissed = get().suggestions.find((s) => s.id === suggestionId);
    await resolveSuggestion(set, suggestionId, () => api.dismissAttachmentRuleSuggestion(accountId, suggestionId));
    if (dismissed) {
      set((state) => ({
        dismissedSuggestions: [
          { ...dismissed, status: 'dismissed' },
          ...withoutSuggestion(state.dismissedSuggestions, suggestionId),
        ],
      }));
    }
  },

  fetchDismissedSuggestions: async (accountId) => {
    try {
      const dismissed = await api.listDismissedAttachmentRuleSuggestions(accountId);
      if (get().suggestionsAccountId === accountId || get().suggestionsAccountId === null) {
        set({ dismissedSuggestions: dismissed });
      }
    } catch (error) {
      set({ error: errorText(error) });
      throw error;
    }
  },

  restoreSuggestion: async (accountId, suggestionId) => {
    let pending: AttachmentRuleSuggestion[];
    try {
      pending = await api.restoreAttachmentRuleSuggestion(accountId, suggestionId);
    } catch (error) {
      set({ error: errorText(error) });
      throw error;
    }
    resolvedSuggestionIds.delete(suggestionId);
    // The restore re-mined: this list is newer than any load in flight.
    suggestionsLoadId++;
    set((state) => ({
      suggestions: pending.filter((s) => !resolvedSuggestionIds.has(s.id)),
      suggestionsAccountId: accountId,
      suggestionsLoading: false,
      dismissedSuggestions: withoutSuggestion(state.dismissedSuggestions, suggestionId),
    }));
  },

  acceptSuggestion: (accountId, suggestionId) =>
    resolveSuggestion(set, suggestionId, () => api.acceptAttachmentRuleSuggestion(accountId, suggestionId)),

  beginRuleApply: (ruleId) => {
    const runId = `${Date.now()}-${++ruleApplyRunCounter}`;
    set((state) => ({
      ruleApplies: { ...state.ruleApplies, [ruleId]: { processed: 0, total: 0, saved: 0, status: 'running', runId } },
    }));
    return runId;
  },

  reportRuleApplyProgress: ({ ruleId, runId, processed, total, saved }) =>
    set((state) => {
      const current = state.ruleApplies[ruleId];
      if (current?.status !== 'running' || current.runId !== runId) return {};
      return { ruleApplies: { ...state.ruleApplies, [ruleId]: { ...current, processed, total, saved } } };
    }),

  finishRuleApply: (ruleId, runId, saved, collected) =>
    set((state) => ({
      ruleApplies: forRun(state.ruleApplies, ruleId, runId, (c) => ({ ...c, saved, collected, status: 'done' })),
    })),

  failRuleApply: (ruleId, runId) =>
    set((state) => ({
      ruleApplies: forRun(state.ruleApplies, ruleId, runId, (c) => ({ ...c, status: 'failed' })),
    })),

  dropRuleApply: (ruleId, runId) =>
    set((state) => ({ ruleApplies: forRun(state.ruleApplies, ruleId, runId, () => null) })),

  fetchAttachments: async (accountId, tag) => {
    const fetchId = get().currentFetchId + 1;
    set({ currentFetchId: fetchId, isLoading: true, attachments: [], selectedAttachment: null, checkedIds: new Set() });

    try {
      const [attachments, totalCount] = await Promise.all([
        api.getAttachments(accountId, tag, PAGE_SIZE, 0),
        api.countAttachments(accountId, tag),
      ]);

      if (get().currentFetchId === fetchId) {
        set({
          attachments,
          totalCount,
          hasMore: attachments.length < totalCount,
          isLoading: false,
        });
      }
    } catch (error) {
      if (get().currentFetchId === fetchId) {
        set({ error: errorText(error), isLoading: false });
      }
    }
  },

  loadMoreAttachments: async (accountId) => {
    const { attachments, hasMore, isLoadingMore, selectedTag } = get();
    if (!hasMore || isLoadingMore) return;

    set({ isLoadingMore: true });
    try {
      const more = await api.getAttachments(accountId, selectedTag, PAGE_SIZE, attachments.length);
      set((state) => ({
        attachments: [...state.attachments, ...more],
        hasMore: state.attachments.length + more.length < state.totalCount,
        isLoadingMore: false,
      }));
    } catch (error) {
      set({ error: errorText(error), isLoadingMore: false });
    }
  },

  selectAttachment: (attachment) => set({ selectedAttachment: attachment }),

  toggleChecked: (id) =>
    set((state) => {
      const next = new Set(state.checkedIds);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { checkedIds: next };
    }),

  toggleCheckAll: () =>
    set((state) => {
      if (state.checkedIds.size === state.attachments.length) {
        return { checkedIds: new Set() };
      }
      return { checkedIds: new Set(state.attachments.map((a) => a.id)) };
    }),

  clearChecked: () => set({ checkedIds: new Set() }),

  setSelectedTag: (tag) => set({ selectedTag: tag }),

  fetchTags: async (accountId) => {
    try {
      const tags = await api.getAttachmentTags(accountId);
      // A tag picked on another account, or whose last attachment went with a
      // deleted rule, would keep filtering the list to nothing with no chip
      // lit to show why: fall back to "All".
      set((state) => ({
        availableTags: tags,
        selectedTag: state.selectedTag !== null && tags.includes(state.selectedTag) ? state.selectedTag : null,
      }));
    } catch (error) {
      console.error('Failed to fetch tags:', error);
    }
  },

  clearError: () => set({ error: null }),

  reset: () =>
    set({
      attachments: [],
      selectedAttachment: null,
      checkedIds: new Set(),
      isLoading: false,
      isLoadingMore: false,
      hasMore: false,
      totalCount: 0,
      error: null,
    }),
}));
