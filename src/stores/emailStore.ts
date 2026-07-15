import { create } from 'zustand';
import type { MailboxView } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { isUnifiedMode, useAccountStore } from '@/stores/accountStore';
import type { ActiveFilter, DraftAttachment, Email, EmailAttachmentMeta, EmailCategory } from '@/types';

const PAGE_SIZE = 50;

/**
 * Pure helper: should the inbox try to load another page after this fetch?
 *
 * Filtered/search endpoints return `totalCount = -1` (intentional — they skip the
 * extra COUNT query for performance). In that case we cannot compare against a
 * known total, so we fall back to a "page-is-full" heuristic: assume there's
 * more iff the backend handed us a full page. When `totalCount` is known and
 * non-negative we use the exact comparison instead.
 *
 * Regression: a bug where Globex filter only showed emails from 2026-03-23
 * onwards came from the fetchEmails path computing `emailsLength < totalCount`
 * with `totalCount = -1`, which collapsed to `false` and pinned hasMore=false
 * forever. Keep this function and its tests in sync.
 */
export function computeHasMore(emailsLength: number, totalCount: number, pageSize: number = PAGE_SIZE): boolean {
  if (totalCount > 0) return emailsLength < totalCount;
  return emailsLength >= pageSize;
}

/**
 * Pure helper: append a freshly-fetched page onto the existing list, dropping
 * any emails whose id is already present.
 *
 * Pagination is offset-based (`offset = emails.length`). If a new email is
 * inserted at the top of the ordering between the initial fetch and a
 * load-more — e.g. a post-send sync pulling the Sent copy to position 0 — every
 * row shifts down by one and the next page re-returns a row already in the
 * list. Appending it blindly yields two React children with the same key, which
 * React warns about and renders incorrectly. Deduplicating on append keeps keys
 * unique regardless of offset drift.
 */
export function appendUniqueEmails(existing: Email[], more: Email[]): Email[] {
  const seen = new Set(existing.map((e) => e.id));
  const additions: Email[] = [];
  for (const email of more) {
    if (seen.has(email.id)) continue;
    seen.add(email.id);
    additions.push(email);
  }
  return additions.length === more.length ? [...existing, ...more] : [...existing, ...additions];
}

/**
 * Pure helper: merge a freshly fetched thread into the one already on screen.
 *
 * The fetched rows are the source of truth for membership and ordering — a
 * row that disappeared (e.g. an optimistic sent copy replaced by the
 * provider's real one during reconciliation) is dropped. But `getThread`
 * returns rows with empty bodies (bodies load lazily), so a refresh must not
 * blank out bodies the user already has expanded: when the fetched body is
 * empty and the existing row has one, the existing body is kept.
 */
export function mergeThreadRefresh(existing: Email[], fetched: Email[]): Email[] {
  const bodies = new Map(existing.filter((e) => e.body).map((e) => [e.id, e.body]));
  return [...fetched]
    .sort((a, b) => a.timestamp - b.timestamp)
    .map((e) => (e.body ? e : { ...e, body: bodies.get(e.id) ?? e.body }));
}

export interface EmailThreadTab {
  type: 'thread';
  id: string;
  threadId: string;
  accountId: string;
  subject: string;
  threadEmails: Email[];
  isLoading: boolean;
  focusEmailId: string | null;
}

export interface AttachmentViewTab {
  type: 'attachment';
  id: string;
  filename: string;
  mimeType: string;
  dataUrl: string;
  isLoading: boolean;
}

export interface ComposeTab {
  type: 'compose';
  id: string;
  accountId: string;
  toAddresses: string[];
  /** Cc recipients. Present when editing a draft that had a Cc list. */
  ccAddresses?: string[];
  subject: string;
  /** Rich-text HTML body. Maximizing from the compose modal hands the
   *  editor's HTML straight to the tab so formatting and inline images
   *  survive the switch. */
  bodyHtml: string;
  /** Backing draft row id when this tab was opened to edit an existing draft.
   *  Auto-save upserts this row instead of creating a new one. */
  draftId?: string;
  /** File-path attachments carried over from the draft being edited, so the tab
   *  can display them, preserve them across auto-saves, and send them. */
  attachments?: DraftAttachment[];
}

export type EmailTab = EmailThreadTab | AttachmentViewTab | ComposeTab;

/**
 * A reply draft the chat just generated for an existing inbound email.
 * `EmailView` consumes this once the matching thread is loaded so the
 * inline `ReplyCompose` opens with the AI body prepended to the quoted
 * template — same shape the "AI Draft" button produces.
 */
export interface PendingChatDraft {
  emailId: string;
  body: string;
}

interface EmailStore {
  emails: Email[];
  selectedEmail: Email | null;
  threadEmails: Email[];
  isLoading: boolean;
  isLoadingMore: boolean;
  isLoadingThread: boolean;
  hasMore: boolean;
  totalCount: number;
  error: string | null;
  searchQuery: string | null;
  focusEmailId: string | null;
  /** True after navigateToEmail — disables category filtering until next explicit inbox action */
  navigationMode: boolean;
  /** One-shot flag — next fetchEmails call is a no-op (used when search results are pre-seeded) */
  skipNextFetch: boolean;
  currentFetchId: number;
  loadMoreLock: boolean;
  tabs: EmailTab[];
  activeTabId: string | null;
  /**
   * Chat-generated reply draft waiting to be opened inside its thread.
   * Set by the chat-tool-effect dispatcher, consumed by `EmailView` once
   * the thread for `emailId` is mounted with its latest email loaded.
   */
  pendingChatDraft: PendingChatDraft | null;
  setPendingChatDraft: (draft: PendingChatDraft) => void;
  consumePendingChatDraft: () => void;
  openTab: (email: Email, focusId?: string) => Promise<void>;
  openAttachmentTab: (meta: EmailAttachmentMeta) => Promise<void>;
  openComposeTab: (
    accountId: string,
    toAddresses?: string[],
    subject?: string,
    bodyHtml?: string,
    opts?: { draftId?: string; ccAddresses?: string[]; attachments?: DraftAttachment[] },
  ) => void;
  closeTab: (tabId: string) => void;
  setActiveTab: (tabId: string | null) => void;
  /** `accountId: null` = unified ("All accounts") view — merged across every
   *  enabled account. Callers translate the UI sentinel via `toQueryAccountId`. */
  fetchEmails: (
    accountId: string | null,
    filter?: ActiveFilter | null,
    selectedCategories?: EmailCategory[],
    silent?: boolean,
    mailbox?: MailboxView,
  ) => Promise<void>;
  loadMoreEmails: (
    accountId: string | null,
    filter?: ActiveFilter | null,
    selectedCategories?: EmailCategory[],
    mailbox?: MailboxView,
  ) => Promise<void>;
  selectEmail: (email: Email | null, focusId?: string) => Promise<void>;
  /**
   * Silently refetch the thread currently on screen (selected pane and/or
   * matching thread tab). Used right after a reply is sent — the backend has
   * already inserted the optimistic Sent row when the send command returns —
   * and when a sync batch lands, so a pending row is transparently swapped
   * for the provider's reconciled copy.
   */
  refreshThread: (accountId: string, threadId: string) => Promise<void>;
  /**
   * Monotonic counter bumped after every successful send. App-level effects
   * watch it to silently refresh the email list (Sent view shows the new
   * mail instantly) without components holding App's fetch closure.
   */
  sentRefreshTick: number;
  bumpSentRefresh: () => void;
  navigateToEmail: (accountId: string, emailId: string) => Promise<void>;
  markAsRead: (emailId: string) => Promise<void>;
  deleteEmail: (emailId: string) => Promise<void>;
  updateEmail: (updated: Email) => void;
  setSearchQuery: (query: string) => void;
  /** Apply a search query with already-fetched results (skips the next fetchEmails call). */
  applySearchResults: (query: string, emails: Email[]) => void;
  clearSearchQuery: () => void;
  clearError: () => void;
  reset: () => void;
}

export const useEmailStore = create<EmailStore>((set, get) => ({
  emails: [],
  selectedEmail: null,
  threadEmails: [],
  isLoading: false,
  isLoadingMore: false,
  isLoadingThread: false,
  hasMore: true,
  totalCount: 0,
  error: null,
  searchQuery: null,
  focusEmailId: null,
  navigationMode: false,
  skipNextFetch: false,
  currentFetchId: 0,
  loadMoreLock: false,
  tabs: [],
  activeTabId: null,
  pendingChatDraft: null,

  setPendingChatDraft: (draft) => set({ pendingChatDraft: draft }),
  consumePendingChatDraft: () => set({ pendingChatDraft: null }),

  openTab: async (email, focusId) => {
    if (!email.isRead) void get().markAsRead(email.id);

    const existing = get().tabs.find((t) => t.id === email.threadId);
    if (existing) {
      set({ activeTabId: email.threadId });
      return;
    }

    const newTab: EmailThreadTab = {
      type: 'thread',
      id: email.threadId,
      threadId: email.threadId,
      accountId: email.accountId,
      subject: email.subject,
      threadEmails: [],
      isLoading: true,
      focusEmailId: focusId ?? null,
    };
    set((state) => ({ tabs: [...state.tabs, newTab], activeTabId: email.threadId }));

    try {
      const [threadEmails, selectedBody] = await Promise.all([
        api.getThread(email.accountId, email.threadId),
        api.getEmailBody(email.accountId, email.id),
      ]);
      threadEmails.sort((a, b) => a.timestamp - b.timestamp);
      const withBody = threadEmails.map((e) => (e.id === email.id ? { ...e, body: selectedBody } : e));
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.type === 'thread' && t.id === email.threadId ? { ...t, threadEmails: withBody, isLoading: false } : t,
        ),
      }));
    } catch {
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.type === 'thread' && t.id === email.threadId ? { ...t, threadEmails: [email], isLoading: false } : t,
        ),
      }));
    }
  },

  openAttachmentTab: async (meta) => {
    const existing = get().tabs.find((t) => t.id === meta.id);
    if (existing) {
      set({ activeTabId: meta.id });
      return;
    }

    const newTab: AttachmentViewTab = {
      type: 'attachment',
      id: meta.id,
      filename: meta.filename,
      mimeType: meta.mimeType,
      dataUrl: '',
      isLoading: true,
    };
    set((state) => ({ tabs: [...state.tabs, newTab], activeTabId: meta.id }));

    try {
      const base64 = await api.fetchEmailAttachmentBytes(meta.accountId, meta.emailId, meta.providerAttachmentId);
      const dataUrl = `data:${meta.mimeType};base64,${base64}`;
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.type === 'attachment' && t.id === meta.id ? { ...t, dataUrl, isLoading: false } : t,
        ),
      }));
    } catch (error) {
      console.error(`Failed to load attachment "${meta.filename}":`, errorText(error));
      // Leave dataUrl empty so AttachmentTabView shows its "load failed" state.
      set((state) => ({
        tabs: state.tabs.map((t) => (t.type === 'attachment' && t.id === meta.id ? { ...t, isLoading: false } : t)),
      }));
    }
  },

  openComposeTab: (accountId, toAddresses = [], subject = '', bodyHtml = '', opts) => {
    const id = `compose-${Date.now()}`;
    const newTab: ComposeTab = {
      type: 'compose',
      id,
      accountId,
      toAddresses,
      ccAddresses: opts?.ccAddresses,
      subject,
      bodyHtml,
      draftId: opts?.draftId,
      attachments: opts?.attachments,
    };
    set((state) => ({ tabs: [...state.tabs, newTab], activeTabId: id }));
  },

  closeTab: (tabId) => {
    const { tabs, activeTabId } = get();
    const idx = tabs.findIndex((t) => t.id === tabId);
    if (idx === -1) return;
    const newTabs = tabs.filter((t) => t.id !== tabId);
    let newActive = activeTabId;
    if (activeTabId === tabId) {
      const next = newTabs[idx] ?? newTabs[idx - 1] ?? null;
      newActive = next?.id ?? null;
    }
    set({ tabs: newTabs, activeTabId: newActive });
  },

  setActiveTab: (tabId) => set({ activeTabId: tabId ?? null }),

  fetchEmails: async (accountId, filter, selectedCategories, silent = false, mailbox) => {
    // Skip if navigateToEmail is in progress — it manages its own fetching
    if (get().navigationMode) return;

    // Skip if results were pre-seeded via applySearchResults
    if (get().skipNextFetch) {
      set({ skipNextFetch: false });
      return;
    }
    // Increment fetch ID to track this operation and cancel stale ones
    const fetchId = get().currentFetchId + 1;
    const { searchQuery } = get();

    // Silent mode: background refresh after sync — never show loading or clear the list.
    // Non-silent: show loading indicator. Clear the email list only when a filter or
    // search is active (because results will be completely different). When switching
    // to an empty inbox with no filter/search, keep whatever is in the list so there
    // is no flash-to-empty between the clear and the fetch completing.
    if (!silent) {
      const shouldClear = Boolean(filter || searchQuery);
      set({
        isLoading: true,
        ...(shouldClear ? { emails: [] } : {}),
        error: null,
        currentFetchId: fetchId,
        navigationMode: false,
        focusEmailId: null,
      });
    } else {
      set({ error: null, currentFetchId: fetchId, navigationMode: false, focusEmailId: null });
    }

    try {
      let emails: Email[];
      let totalCount: number;

      if (searchQuery) {
        // Search mode — use search API, preserve backend result order (relevance for RAG)
        const result = await api.searchEmails(accountId, searchQuery, true, selectedCategories);
        emails = result.emails;
        totalCount = result.emails.length;
      } else if (filter) {
        const domain = filter.type === 'domain' ? filter.value : undefined;
        const senderEmail = filter.type === 'sender' ? filter.value : undefined;
        const isTagFilter = ['priority', 'intent', 'topic', 'company'].includes(filter.type);
        const tagType = isTagFilter ? filter.type : undefined;
        const tagValue = isTagFilter ? filter.value : undefined;
        const attachmentExt = filter.type === 'attachment_ext' ? filter.value : undefined;
        const result = await api.getFilteredEmails(
          accountId,
          domain,
          senderEmail,
          tagType,
          tagValue,
          PAGE_SIZE,
          0,
          attachmentExt,
        );
        emails = result.emails;
        totalCount = result.totalCount;
      } else {
        [emails, totalCount] = await Promise.all([
          api.getEmails(accountId, PAGE_SIZE, 0, mailbox),
          api.getEmailCount(accountId),
        ]);
      }

      // Only update state if this is still the current fetch operation
      if (get().currentFetchId === fetchId) {
        set({
          emails,
          totalCount,
          isLoading: false,
          hasMore: computeHasMore(emails.length, totalCount),
          loadMoreLock: false,
        });
      }
    } catch (error) {
      // Only update state if this is still the current fetch operation
      if (get().currentFetchId === fetchId) {
        set({ error: errorText(error), isLoading: false });
      }
    }
  },

  loadMoreEmails: async (accountId, filter, _selectedCategories, mailbox) => {
    const { isLoadingMore, hasMore, emails, totalCount, loadMoreLock, currentFetchId, searchQuery } = get();

    // Don't load more when in search mode — search returns all results at once
    if (searchQuery) return;

    // Use both isLoadingMore flag and lock to prevent concurrent operations
    if (isLoadingMore || !hasMore || loadMoreLock) {
      return;
    }

    // Acquire lock and set loading state
    set({ isLoadingMore: true, loadMoreLock: true });

    try {
      // Capture the offset at the start to avoid race conditions
      const offset = emails.length;

      let moreEmails: Email[];
      if (filter) {
        const isTagFilter = ['priority', 'intent', 'topic', 'company'].includes(filter.type);
        const result = await api.getFilteredEmails(
          accountId,
          filter.type === 'domain' ? filter.value : undefined,
          filter.type === 'sender' ? filter.value : undefined,
          isTagFilter ? filter.type : undefined,
          isTagFilter ? filter.value : undefined,
          PAGE_SIZE,
          offset,
          filter.type === 'attachment_ext' ? filter.value : undefined,
        );
        moreEmails = result.emails;
      } else {
        moreEmails = await api.getEmails(accountId, PAGE_SIZE, offset, mailbox);
      }

      // Check if fetch ID changed (account switched during load)
      if (get().currentFetchId !== currentFetchId) {
        set({ isLoadingMore: false, loadMoreLock: false });
        return;
      }

      const newTotal = emails.length + moreEmails.length;

      set((state) => ({
        emails: appendUniqueEmails(state.emails, moreEmails),
        isLoadingMore: false,
        loadMoreLock: false,
        // Same hasMore logic as fetchEmails — see computeHasMore for the
        // totalCount=-1 fallback used by filtered/search endpoints.
        hasMore: totalCount > 0 ? newTotal < totalCount : computeHasMore(moreEmails.length, totalCount),
      }));
    } catch (error) {
      console.error('Failed to load more emails:', error);
      set({ isLoadingMore: false, loadMoreLock: false, error: errorText(error) });
    }
  },

  selectEmail: async (email, focusId) => {
    if (!email) {
      set({ selectedEmail: null, threadEmails: [], focusEmailId: null });
      return;
    }

    if (!email.isRead) void get().markAsRead(email.id);

    set({ selectedEmail: email, threadEmails: [], isLoadingThread: true, focusEmailId: focusId ?? null });

    try {
      // Fetch thread metadata and the selected email's body in parallel.
      // The selected email is always expanded first; pre-loading its body avoids
      // a visible spinner on the most important message.
      const [threadEmails, selectedBody] = await Promise.all([
        api.getThread(email.accountId, email.threadId),
        api.getEmailBody(email.accountId, email.id),
      ]);
      threadEmails.sort((a, b) => a.timestamp - b.timestamp);
      const withBody = threadEmails.map((e) => (e.id === email.id ? { ...e, body: selectedBody } : e));
      set({ threadEmails: withBody, isLoadingThread: false });
    } catch (error) {
      // Fall back to showing just the selected email, but surface the error
      set({ threadEmails: [email], isLoadingThread: false, error: errorText(error) });
    }
  },

  sentRefreshTick: 0,
  bumpSentRefresh: () => set((state) => ({ sentRefreshTick: state.sentRefreshTick + 1 })),

  refreshThread: async (accountId, threadId) => {
    try {
      const fetched = await api.getThread(accountId, threadId);

      set((state) => {
        const next: Partial<EmailStore> = {
          tabs: state.tabs.map((t) =>
            t.type === 'thread' && t.threadId === threadId && t.accountId === accountId
              ? { ...t, threadEmails: mergeThreadRefresh(t.threadEmails, fetched) }
              : t,
          ),
        };
        // Guard against the selection having moved while the fetch was in
        // flight: only replace the selected pane when it still shows this
        // thread.
        if (state.selectedEmail?.threadId === threadId && state.selectedEmail?.accountId === accountId) {
          next.threadEmails = mergeThreadRefresh(state.threadEmails, fetched);
        }
        return next;
      });
    } catch (error) {
      // Non-fatal: the thread simply keeps its current contents.
      console.error('Failed to refresh thread:', error);
    }
  },

  navigateToEmail: async (accountId, emailId) => {
    const fetchId = get().currentFetchId + 1;
    set({
      currentFetchId: fetchId,
      isLoading: true,
      isLoadingThread: true,
      emails: [],
      selectedEmail: null,
      threadEmails: [],
      searchQuery: null,
      focusEmailId: emailId,
      navigationMode: true,
    });

    try {
      // `accountId` is the email's OWNING account (required by getEmailById /
      // getThread). The surrounding LIST is scoped to the current view: in
      // unified ("All accounts") mode the position and page span every
      // enabled account so the focused email lands in the merged list.
      const listAccountId = isUnifiedMode(useAccountStore.getState().activeAccountId) ? null : accountId;
      const [email, position, totalCount] = await Promise.all([
        api.getEmailById(accountId, emailId),
        api.getEmailInboxPosition(listAccountId, emailId),
        api.getEmailCount(listAccountId),
      ]);

      if (get().currentFetchId !== fetchId) return;

      // Load from offset 0 through the target email so the full list
      // is scrollable from the top. Add a page of buffer below.
      const limit = position + PAGE_SIZE;
      const emails = await api.getEmails(listAccountId, limit, 0);

      if (get().currentFetchId !== fetchId) return;

      if (!email.isRead) void get().markAsRead(email.id);

      set({
        emails,
        totalCount,
        isLoading: false,
        hasMore: emails.length < totalCount,
        loadMoreLock: false,
        selectedEmail: email,
        threadEmails: [],
        isLoadingThread: true,
      });

      // Load the thread (with body pre-fetched for the focused email)
      try {
        const [threadEmails, focusedBody] = await Promise.all([
          api.getThread(email.accountId, email.threadId),
          api.getEmailBody(email.accountId, email.id),
        ]);
        threadEmails.sort((a, b) => a.timestamp - b.timestamp);
        const withBody = threadEmails.map((e) => (e.id === email.id ? { ...e, body: focusedBody } : e));
        set({ threadEmails: withBody, isLoadingThread: false });
      } catch {
        set({ threadEmails: [email], isLoadingThread: false });
      }
    } catch (error) {
      if (get().currentFetchId === fetchId) {
        set({ error: errorText(error), isLoading: false, isLoadingThread: false, navigationMode: false });
      }
    }
  },

  markAsRead: async (emailId) => {
    try {
      await api.markAsRead(emailId);
      set((state) => ({
        emails: state.emails.map((e) => (e.id === emailId ? { ...e, isRead: true } : e)),
        threadEmails: state.threadEmails.map((e) => (e.id === emailId ? { ...e, isRead: true } : e)),
        selectedEmail:
          state.selectedEmail?.id === emailId ? { ...state.selectedEmail, isRead: true } : state.selectedEmail,
        tabs: state.tabs.map((t) =>
          t.type === 'thread'
            ? { ...t, threadEmails: t.threadEmails.map((e) => (e.id === emailId ? { ...e, isRead: true } : e)) }
            : t,
        ),
      }));
    } catch (error) {
      console.error('Failed to mark as read:', error);
    }
  },

  deleteEmail: async (emailId) => {
    await api.deleteEmail(emailId);
    set((state) => ({
      emails: state.emails.filter((e) => e.id !== emailId),
      threadEmails: state.threadEmails.filter((e) => e.id !== emailId),
      selectedEmail: state.selectedEmail?.id === emailId ? null : state.selectedEmail,
      totalCount: Math.max(0, state.totalCount - 1),
      tabs: state.tabs.map((t) =>
        t.type === 'thread' ? { ...t, threadEmails: t.threadEmails.filter((e) => e.id !== emailId) } : t,
      ),
    }));
  },

  updateEmail: (updated) => {
    set((state) => ({
      emails: state.emails.map((e) => (e.id === updated.id ? updated : e)),
      threadEmails: state.threadEmails.map((e) => (e.id === updated.id ? updated : e)),
      selectedEmail: state.selectedEmail?.id === updated.id ? updated : state.selectedEmail,
      tabs: state.tabs.map((t) =>
        t.type === 'thread'
          ? { ...t, threadEmails: t.threadEmails.map((e) => (e.id === updated.id ? updated : e)) }
          : t,
      ),
    }));
  },

  setSearchQuery: (query) => set({ searchQuery: query }),
  applySearchResults: (query, emails) => {
    // Pre-seed emails and mark the next effect-triggered fetchEmails as a no-op.
    // Also bump fetchId so any already-in-flight fetch's response is discarded.
    set((state) => ({
      searchQuery: query,
      emails,
      totalCount: emails.length,
      hasMore: false,
      isLoading: false,
      skipNextFetch: true,
      currentFetchId: state.currentFetchId + 1,
    }));
  },
  clearSearchQuery: () => set({ searchQuery: null }),

  clearError: () => set({ error: null }),

  reset: () =>
    set({
      emails: [],
      selectedEmail: null,
      threadEmails: [],
      isLoading: false,
      isLoadingMore: false,
      isLoadingThread: false,
      hasMore: true,
      totalCount: 0,
      error: null,
      searchQuery: null,
      focusEmailId: null,
      navigationMode: false,
      skipNextFetch: false,
      loadMoreLock: false,
      tabs: [],
      activeTabId: null,
      pendingChatDraft: null,
      sentRefreshTick: 0,
    }),
}));
