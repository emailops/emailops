import { create } from 'zustand';
import { i18n } from '@/i18n';
import type { EmailAttachment, MailboxView, ThreadAction, ThreadRef } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { normalizeMimeType } from '@/lib/mimeType';
import { isUnifiedMode, useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';
import { useToastStore } from '@/stores/toastStore';
import type { ActiveFilter, DraftAttachment, Email, EmailAttachmentMeta, EmailCategory } from '@/types';

const PAGE_SIZE = 50;
/** Ceiling for a background refetch that keeps the loaded pages (see refetchLimit). */
const MAX_REFETCH = 500;

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
 * Pure helper: should we keep paging after a load-more returned `pageLength`
 * rows, bringing the list to `newTotal`?
 *
 * A short page is proof the backend has nothing left, and it outranks
 * `totalCount` — the two can legitimately describe different sets. The mailbox
 * views (Sent / Spam / Trash / custom folder) list one mailbox while
 * `totalCount` counts inbox threads, so trusting the total alone made the list
 * re-request an empty page forever with the spinner stuck on screen.
 */
export function computeHasMoreAfterPage(
  newTotal: number,
  pageLength: number,
  totalCount: number,
  pageSize: number = PAGE_SIZE,
): boolean {
  if (pageLength < pageSize) return false;
  if (totalCount > 0) return newTotal < totalCount;
  return true;
}

/**
 * Pure helper: how many rows a refetch has to ask for.
 *
 * Every refetch starts at offset 0 and *replaces* the list, so requesting a
 * single page silently undoes the user's paging. That is invisible on a normal
 * refetch (the list is being swapped anyway) but wrong for the background ones —
 * a sync batch, the refresh after a send, an account being enabled — which are
 * supposed to be transparent: a user five pages down would drop back to 50 rows
 * mid-read, with the container scrolled past the end of the shortened list.
 *
 * Capped, because a very deep list would otherwise turn every sync batch into a
 * progressively larger query.
 */
export function refetchLimit(loadedCount: number, pageSize: number = PAGE_SIZE, cap: number = MAX_REFETCH): number {
  return Math.max(pageSize, Math.min(loadedCount, cap));
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

/**
 * Drop one email from every list slice — inbox list, thread view, open
 * thread tabs, selection, total count. Shared by deleteEmail and moveEmail,
 * which both make the email vanish from its current view.
 */
export function removeEmailFromSlices(
  state: Pick<EmailStore, 'emails' | 'threadEmails' | 'selectedEmail' | 'totalCount' | 'tabs'>,
  emailId: string,
): Pick<EmailStore, 'emails' | 'threadEmails' | 'selectedEmail' | 'totalCount' | 'tabs'> {
  return {
    emails: state.emails.filter((e) => e.id !== emailId),
    threadEmails: state.threadEmails.filter((e) => e.id !== emailId),
    selectedEmail: state.selectedEmail?.id === emailId ? null : state.selectedEmail,
    totalCount: Math.max(0, state.totalCount - 1),
    tabs: state.tabs.map((t) =>
      t.type === 'thread' ? { ...t, threadEmails: t.threadEmails.filter((e) => e.id !== emailId) } : t,
    ),
  };
}

// ── Thread actions: pure reducers ───────────────────────────────────────────

/** Map key for one conversation — thread ids are only unique per account. */
export function threadKey(accountId: string, threadId: string): string {
  return `${accountId}\u0000${threadId}`;
}

const keyOf = (e: Pick<Email, 'accountId' | 'threadId'>) => threadKey(e.accountId, e.threadId);

/** The conversation an email belongs to, as the thread actions take it. */
export function threadRefOf(email: Pick<Email, 'accountId' | 'threadId'>): ThreadRef {
  return { accountId: email.accountId, threadId: email.threadId };
}

/** What the list on screen shows: a mailbox view, or search results. */
export type ListScope = MailboxView | 'search';

type ThreadSlices = Pick<EmailStore, 'emails' | 'threadEmails' | 'selectedEmail' | 'totalCount' | 'tabs'>;

/** A conversation is starred when any of its messages is. */
export function isThreadStarred(threadEmails: readonly Email[]): boolean {
  return threadEmails.some((e) => e.isStarred);
}

/** A conversation is unread when any of its messages is. */
export function isThreadUnread(threadEmails: readonly Email[]): boolean {
  return threadEmails.some((e) => !e.isRead);
}

/**
 * Mirror of the backend planner on an open thread's messages: setting read or
 * clearing the star touches every message, marking unread or starring touches
 * the latest one (the thread then reads unread / starred).
 */
function patchThreadMessages(messages: Email[], flag: 'isRead' | 'isStarred', value: boolean): Email[] {
  const wholeThread = flag === 'isRead' ? value : !value;
  if (wholeThread) return messages.map((e) => (e[flag] === value ? e : { ...e, [flag]: value }));
  if (messages.length === 0) return messages;
  let latest = 0;
  messages.forEach((e, i) => {
    const l = messages[latest];
    if (e.timestamp > l.timestamp || (e.timestamp === l.timestamp && e.id > l.id)) latest = i;
  });
  return messages.map((e, i) => (i === latest ? { ...e, [flag]: value } : e));
}

/**
 * Pure: the optimistic read/star change for the conversations in `keys` — the
 * list rows (which stand for their thread), the selection and any open copy
 * of the thread.
 */
export function applyThreadFlag<S extends ThreadSlices>(
  state: S,
  keys: ReadonlySet<string>,
  flag: 'isRead' | 'isStarred',
  value: boolean,
): S {
  const row = (e: Email) => (keys.has(keyOf(e)) && e[flag] !== value ? { ...e, [flag]: value } : e);
  const openThread = state.threadEmails[0] ?? state.selectedEmail;
  const threadEmails =
    openThread && keys.has(keyOf(openThread))
      ? patchThreadMessages(state.threadEmails, flag, value)
      : state.threadEmails;
  const selectedEmail = state.selectedEmail
    ? (threadEmails.find((e) => e.id === state.selectedEmail?.id) ?? row(state.selectedEmail))
    : null;
  return {
    ...state,
    emails: state.emails.map(row),
    threadEmails,
    selectedEmail,
    tabs: state.tabs.map((t) =>
      t.type === 'thread' && keys.has(threadKey(t.accountId, t.threadId))
        ? { ...t, threadEmails: patchThreadMessages(t.threadEmails, flag, value) }
        : t,
    ),
  };
}

/** Pure: the conversations in `keys` leave the list (and the selection). */
export function removeThreads<S extends ThreadSlices>(state: S, keys: ReadonlySet<string>): S {
  const emails = state.emails.filter((e) => !keys.has(keyOf(e)));
  const deselect = state.selectedEmail !== null && keys.has(keyOf(state.selectedEmail));
  return {
    ...state,
    emails,
    totalCount: Math.max(0, state.totalCount - (state.emails.length - emails.length)),
    selectedEmail: deselect ? null : state.selectedEmail,
    threadEmails: deselect ? [] : state.threadEmails,
  };
}

/**
 * Pure: undo an optimistic change for the conversations in `failed` only. Their
 * rows come back from `before`, in their old place; every other row keeps what
 * happened to it since (other threads' changes, a refetch).
 */
export function restoreThreads<S extends ThreadSlices>(current: S, before: S, failed: ReadonlySet<string>): S {
  if (failed.size === 0) return current;
  const now = new Map(current.emails.map((e) => [e.id, e]));
  const emails: Email[] = [];
  for (const e of before.emails) {
    if (failed.has(keyOf(e))) emails.push(e);
    else {
      const kept = now.get(e.id);
      if (kept) emails.push(kept);
    }
  }
  const known = new Set(before.emails.map((e) => e.id));
  for (const e of current.emails) if (!known.has(e.id) && !failed.has(keyOf(e))) emails.push(e);
  const restoreSelection = before.selectedEmail !== null && failed.has(keyOf(before.selectedEmail));
  return {
    ...current,
    emails,
    totalCount: Math.max(0, current.totalCount + (emails.length - current.emails.length)),
    selectedEmail: restoreSelection ? before.selectedEmail : current.selectedEmail,
    threadEmails: restoreSelection ? before.threadEmails : current.threadEmails,
    tabs: current.tabs.map((t) => {
      if (t.type !== 'thread' || !failed.has(threadKey(t.accountId, t.threadId))) return t;
      const old = before.tabs.find((b) => b.id === t.id);
      return old ?? t;
    }),
  };
}

/** Pure: whether `action` takes a conversation out of the list on screen. */
export function leavesList(action: ThreadAction, list: ListScope): boolean {
  switch (action) {
    case 'archive':
      return list === 'inbox';
    case 'moveToInbox':
      return list === 'archive' || list.startsWith('folder:');
    case 'unstar':
      return list === 'starred';
    default:
      return false;
  }
}

/**
 * Stale-response guards for thread loads. Every selection (and every reset)
 * takes a new id; a thread fetch writes only while its id is still the latest,
 * so a slow response for a previous selection cannot replace the thread of the
 * email on screen — which would also point Reply at the wrong message.
 */
let threadRequestSeq = 0;
/** Latest load id per open thread tab (keyed by tab id). A tab closed and
 *  reopened, or wiped by `reset`, must not receive the older load's result. */
const tabLoadIds = new Map<string, number>();

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
  /** Files attached in the compose modal before it was opened in a tab
   *  (base64, not yet on any draft row). */
  fileAttachments?: EmailAttachment[];
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
  /** True only while navigateToEmail is loading its list — fetchEmails stands
   *  aside so it cannot overwrite the list being built around the focused email. */
  navigationInFlight: boolean;
  /** One-shot flag — next fetchEmails call is a no-op (used when search results are pre-seeded) */
  skipNextFetch: boolean;
  currentFetchId: number;
  loadMoreLock: boolean;
  /** A load-more failed: paging stays off until the list is fetched again, so
   *  the scroll-triggered load-more does not retry in a tight loop. */
  loadMoreFailed: boolean;
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
    opts?: {
      draftId?: string;
      ccAddresses?: string[];
      attachments?: DraftAttachment[];
      fileAttachments?: EmailAttachment[];
    },
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
  /** `markRead: false` opens the email without marking it read (the split
   *  layout's automatic first selection). */
  selectEmail: (email: Email | null, focusId?: string, opts?: { markRead?: boolean }) => Promise<void>;
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
  /** What the list on screen shows, recorded by `fetchEmails` — decides
   *  whether a thread action takes rows out of it. */
  listScope: ListScope;
  /**
   * Thread actions, for one conversation or many (bulk selection). Each
   * updates the UI optimistically, sends the change to the account, and rolls
   * back — with an error toast — exactly the conversations the backend could
   * not change. Resolve once the backend answered; they never reject.
   *
   * Marking unread leaves the open thread, so it is not marked read again the
   * moment it is looked at. Archive and move-to-inbox drop the conversation
   * from a list it no longer belongs to.
   */
  setThreadsRead: (threads: ThreadRef[], read: boolean) => Promise<void>;
  setThreadsStarred: (threads: ThreadRef[], starred: boolean) => Promise<void>;
  archiveThreads: (threads: ThreadRef[]) => Promise<void>;
  moveThreadsToInbox: (threads: ThreadRef[]) => Promise<void>;
  deleteEmail: (emailId: string) => Promise<void>;
  /** Move an email to the inbox or a custom folder (IMAP accounts only).
   *  Throws on failure so callers can surface the error. */
  moveEmail: (accountId: string, emailId: string, targetMailbox: MailboxView) => Promise<void>;
  updateEmail: (updated: Email) => void;
  setSearchQuery: (query: string) => void;
  /** Apply a search query with already-fetched results (skips the next fetchEmails call). */
  applySearchResults: (query: string, emails: Email[]) => void;
  clearSearchQuery: () => void;
  clearError: () => void;
  reset: () => void;
  /** Account key the list was last reset for (see `resetForAccount`). */
  resetAccountKey: string | null;
  /**
   * Clear the list, selection and tabs because the app switched to
   * `accountKey` — a no-op when it was already reset for that key. The App
   * effect that calls this re-runs whenever the account list reloads (saving
   * account settings, reordering, re-auth), and an unconditional reset there
   * closed every open tab while the account stayed the same.
   */
  resetForAccount: (accountKey: string) => void;
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
  navigationInFlight: false,
  skipNextFetch: false,
  currentFetchId: 0,
  loadMoreLock: false,
  loadMoreFailed: false,
  tabs: [],
  activeTabId: null,
  pendingChatDraft: null,
  resetAccountKey: null,
  listScope: 'inbox',

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
    const loadId = ++threadRequestSeq;
    tabLoadIds.set(email.threadId, loadId);
    const isCurrentLoad = () => tabLoadIds.get(email.threadId) === loadId;

    try {
      const [threadEmails, selectedBody] = await Promise.all([
        api.getThread(email.accountId, email.threadId),
        api.getEmailBody(email.accountId, email.id),
      ]);
      if (!isCurrentLoad()) return;
      threadEmails.sort((a, b) => a.timestamp - b.timestamp);
      const withBody = threadEmails.map((e) => (e.id === email.id ? { ...e, body: selectedBody } : e));
      set((state) => ({
        tabs: state.tabs.map((t) =>
          t.type === 'thread' && t.id === email.threadId ? { ...t, threadEmails: withBody, isLoading: false } : t,
        ),
      }));
    } catch {
      if (!isCurrentLoad()) return;
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

    // Sender-declared: normalized before it picks the viewer (and its
    // sandbox) or is interpolated into the data: URL.
    const mimeType = normalizeMimeType(meta.mimeType);
    const newTab: AttachmentViewTab = {
      type: 'attachment',
      id: meta.id,
      filename: meta.filename,
      mimeType,
      dataUrl: '',
      isLoading: true,
    };
    set((state) => ({ tabs: [...state.tabs, newTab], activeTabId: meta.id }));

    try {
      const base64 = await api.fetchEmailAttachmentBytes(meta.accountId, meta.emailId, meta.providerAttachmentId);
      const dataUrl = `data:${mimeType};base64,${base64}`;
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
      fileAttachments: opts?.fileAttachments,
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

  fetchEmails: async (accountId, filter, _selectedCategories, silent = false, mailbox) => {
    // Skip while navigateToEmail is loading — it manages its own fetching.
    // Not on navigationMode: that flag outlives the navigation (it keeps the
    // category filter off the navigated list), and gating on it swallowed
    // every later search or filter until the account changed.
    if (get().navigationInFlight) return;

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

    set({ listScope: searchQuery ? 'search' : filter ? 'inbox' : (mailbox ?? 'inbox') });

    // A background refresh is meant to be transparent, so it has to come back
    // with everything the user had already paged in — see refetchLimit.
    const limit = silent ? refetchLimit(get().emails.length) : PAGE_SIZE;

    try {
      let emails: Email[];
      let totalCount: number;

      if (searchQuery) {
        // Search mode — use search API, preserve backend result order (relevance for RAG)
        const result = await api.searchEmails(accountId, searchQuery, true);
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
          limit,
          0,
          attachmentExt,
        );
        emails = result.emails;
        totalCount = result.totalCount;
      } else {
        [emails, totalCount] = await Promise.all([
          api.getEmails(accountId, limit, 0, mailbox),
          // Same mailbox as the list — counting the inbox while listing Sent
          // made hasMore compare two different sets.
          api.getEmailCount(accountId, mailbox),
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
          loadMoreFailed: false,
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
    const { isLoadingMore, hasMore, emails, totalCount, loadMoreLock, loadMoreFailed, currentFetchId, searchQuery } =
      get();

    // Don't load more when in search mode — search returns all results at once
    if (searchQuery) return;

    // Use both isLoadingMore flag and lock to prevent concurrent operations
    if (isLoadingMore || !hasMore || loadMoreLock || loadMoreFailed) {
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
        hasMore: computeHasMoreAfterPage(newTotal, moreEmails.length, totalCount),
      }));
    } catch (error) {
      console.error('Failed to load more emails:', error);
      set({ isLoadingMore: false, loadMoreLock: false, loadMoreFailed: true, error: errorText(error) });
    }
  },

  selectEmail: async (email, focusId, opts) => {
    const requestId = ++threadRequestSeq;
    if (!email) {
      set({ selectedEmail: null, threadEmails: [], focusEmailId: null });
      return;
    }

    if (!email.isRead && opts?.markRead !== false) void get().markAsRead(email.id);

    set({ selectedEmail: email, threadEmails: [], isLoadingThread: true, focusEmailId: focusId ?? null });

    try {
      // Fetch thread metadata and the selected email's body in parallel.
      // The selected email is always expanded first; pre-loading its body avoids
      // a visible spinner on the most important message.
      const [threadEmails, selectedBody] = await Promise.all([
        api.getThread(email.accountId, email.threadId),
        api.getEmailBody(email.accountId, email.id),
      ]);
      if (threadRequestSeq !== requestId) return;
      threadEmails.sort((a, b) => a.timestamp - b.timestamp);
      const withBody = threadEmails.map((e) => (e.id === email.id ? { ...e, body: selectedBody } : e));
      set({ threadEmails: withBody, isLoadingThread: false });
    } catch (error) {
      if (threadRequestSeq !== requestId) return;
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
    // The navigation replaces the selection: a thread still loading for the
    // previous one must not land.
    threadRequestSeq++;
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
      navigationInFlight: true,
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

      const requestId = ++threadRequestSeq;
      set({
        emails,
        totalCount,
        isLoading: false,
        hasMore: emails.length < totalCount,
        loadMoreLock: false,
        navigationInFlight: false,
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
        if (threadRequestSeq !== requestId) return;
        threadEmails.sort((a, b) => a.timestamp - b.timestamp);
        const withBody = threadEmails.map((e) => (e.id === email.id ? { ...e, body: focusedBody } : e));
        set({ threadEmails: withBody, isLoadingThread: false });
      } catch {
        if (threadRequestSeq !== requestId) return;
        set({ threadEmails: [email], isLoadingThread: false });
      }
    } catch (error) {
      if (get().currentFetchId === fetchId) {
        set({
          error: errorText(error),
          isLoading: false,
          isLoadingThread: false,
          navigationMode: false,
          navigationInFlight: false,
        });
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

  setThreadsRead: (threads, read) =>
    runThreadAction(threads, read ? 'markRead' : 'markUnread', (state, keys) => {
      const patched = applyThreadFlag(state, keys, 'isRead', read);
      if (read || !patched.selectedEmail || !keys.has(keyOf(patched.selectedEmail))) return patched;
      // Leave the thread (as Gmail returns to the list): looking at it again
      // right away would mark it read.
      return { ...patched, selectedEmail: null, threadEmails: [] };
    }),

  setThreadsStarred: (threads, starred) =>
    runThreadAction(threads, starred ? 'star' : 'unstar', (state, keys) =>
      applyThreadFlag(state, keys, 'isStarred', starred),
    ),

  archiveThreads: (threads) =>
    runThreadAction(threads, 'archive', (state, keys) => ({
      ...state,
      emails: state.emails.map((e) => (keys.has(keyOf(e)) && e.mailbox === 'inbox' ? { ...e, mailbox: 'archive' } : e)),
    })),

  moveThreadsToInbox: (threads) =>
    runThreadAction(threads, 'moveToInbox', (state, keys) => ({
      ...state,
      emails: state.emails.map((e) =>
        keys.has(keyOf(e)) && (e.mailbox === 'archive' || e.mailbox.startsWith('folder:'))
          ? { ...e, mailbox: 'inbox' }
          : e,
      ),
    })),

  deleteEmail: async (emailId) => {
    await api.deleteEmail(emailId);
    set((state) => removeEmailFromSlices(state, emailId));
  },

  moveEmail: async (accountId, emailId, targetMailbox) => {
    await api.moveEmail(accountId, emailId, targetMailbox);
    // The email left its current mailbox — drop it from every visible slice;
    // the target folder view picks it up on its next fetch.
    set((state) => removeEmailFromSlices(state, emailId));
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
      // The bumped fetchId makes an in-flight navigateToEmail bail out
      // before it clears its own flag; the list is ours now.
      navigationInFlight: false,
    }));
  },
  clearSearchQuery: () => set({ searchQuery: null }),

  clearError: () => set({ error: null }),

  resetForAccount: (accountKey) => {
    if (get().resetAccountKey === accountKey) return;
    get().reset();
    set({ resetAccountKey: accountKey });
  },

  reset: () => {
    // Invalidate the thread loads still in flight: none of them belongs to
    // what comes after the reset (typically another account). `currentFetchId`
    // is deliberately left alone — the list fetch for the new account is
    // usually already in flight when the account effect resets the store, and
    // any older list fetch is superseded by it.
    threadRequestSeq++;
    tabLoadIds.clear();
    set({
      resetAccountKey: null,
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
      navigationInFlight: false,
      skipNextFetch: false,
      loadMoreLock: false,
      loadMoreFailed: false,
      tabs: [],
      activeTabId: null,
      pendingChatDraft: null,
      sentRefreshTick: 0,
      listScope: 'inbox',
    });
  },
}));

/**
 * Shared body of the thread actions: optimistic `patch` (plus removal from a
 * list the threads no longer belong to), the backend call, then rollback of
 * whatever failed, with a log line and one toast.
 */
async function runThreadAction(
  threads: ThreadRef[],
  action: ThreadAction,
  patch: (state: EmailStore, keys: ReadonlySet<string>) => EmailStore,
): Promise<void> {
  if (threads.length === 0) return;
  const keys = new Set(threads.map((t) => threadKey(t.accountId, t.threadId)));
  const before = useEmailStore.getState();
  const patched = patch(before, keys);
  useEmailStore.setState(leavesList(action, before.listScope) ? removeThreads(patched, keys) : patched);

  let failed: ReadonlySet<string>;
  let detail: string;
  try {
    const report = await api.applyThreadAction(threads, action);
    failed = new Set(report.failed.map((f) => threadKey(f.accountId, f.threadId)));
    detail = report.failed.length > 0 ? errorText(report.failed[0]) : '';
  } catch (error) {
    failed = keys;
    detail = errorText(error);
  }
  if (failed.size === 0) return;

  useEmailStore.setState((current) => restoreThreads(current, before, failed));
  const message = i18n.t(`inbox:threadActions.failed.${action}`, { count: failed.size, detail });
  useLogStore.getState().addLog('error', 'sync', message);
  useToastStore.getState().addToast({ message });
}
