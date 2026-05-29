import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Attachment, AttachmentRule } from '@/types';

const PAGE_SIZE = 50;

interface AttachmentStore {
  // Rules
  rules: AttachmentRule[];
  isLoadingRules: boolean;

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
    ruleId: string,
    name: string,
    senderEmailPattern: string | null,
    subjectPattern: string | null,
    filenamePattern: string | null,
    tags: string[],
    enabled: boolean,
  ) => Promise<AttachmentRule>;
  deleteRule: (ruleId: string, accountId: string) => Promise<void>;

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

export const useAttachmentStore = create<AttachmentStore>((set, get) => ({
  rules: [],
  isLoadingRules: false,
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

  updateRule: async (ruleId, name, senderEmailPattern, subjectPattern, filenamePattern, tags, enabled) => {
    const rule = await api.updateAttachmentRule(
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
      set({ availableTags: tags });
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
