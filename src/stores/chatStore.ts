// Chat-with-your-emails store.
//
// Streams token-level updates from the backend into `messages`. The backend is
// the source of truth — conversations and final message content live in SQLite;
// the store holds only the view state for the active account/conversation.

import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type {
  ChatConversation,
  ChatMessage,
  ChatPhase,
  ChatPhaseEvent,
  ChatRenamedEvent,
  ChatSourcesEvent,
  ChatStreamEvent,
  ChatTraceEvent,
  EmailCategory,
} from '@/types';

/** Preference key shared with the Rust backend (`commands/chat.rs`). */
const CATEGORIES_PREF_KEY = 'chat.default_categories';
/** Order the checkbox list is rendered in. Primary first, noisier ones last. */
export const CHAT_CATEGORY_ORDER: EmailCategory[] = ['primary', 'updates', 'promotions', 'social', 'forums'];
/** Default filter — matches `DEFAULT_RAG_CATEGORIES` in services/chat.rs. */
const DEFAULT_CATEGORIES: EmailCategory[] = ['primary'];

function parseCategoriesPref(raw: string | null | undefined): EmailCategory[] {
  if (!raw) return [...DEFAULT_CATEGORIES];
  const valid = new Set<string>(CHAT_CATEGORY_ORDER);
  const parsed = raw
    .split(',')
    .map((t) => t.trim().toLowerCase())
    .filter((t) => valid.has(t)) as EmailCategory[];
  // If the stored value is somehow empty/corrupt, fall back to the default
  // so the user never ends up with a broken "search nothing" filter.
  return parsed.length > 0 ? parsed : [...DEFAULT_CATEGORIES];
}

interface ChatStore {
  conversations: ChatConversation[];
  activeConversationId: string | null;
  /**
   * Turns still streaming in a conversation that is not on screen.
   *
   * The backend keeps generating after you navigate away, but its tokens used
   * to be dropped by the active-conversation guard, and the answer is only
   * persisted when the turn ends. Returning mid-flight therefore showed an
   * empty bubble with no progress — and looked fixed on the next visit purely
   * because the turn had finished by then. Buffering here lets the answer and
   * its status be restored on return.
   */
  backgroundTurns: Record<string, { messageId: string; content: string; phase: ChatPhase | null; done: boolean }>;
  /** Last conversation open per account, this session only. See `selectAccount`. */
  lastConversationByAccount: Record<string, string>;
  /** Account chat is currently answering from. */
  currentAccountId: string | null;
  messages: ChatMessage[];
  /** id of the assistant message currently receiving tokens, if any */
  streamingMessageId: string | null;
  /** Coarse processing stage of the in-flight turn (routing → retrieving →
   *  running tools → generating). Null when nothing is streaming. Drives the
   *  bubble's "Processing…" status before the first answer token arrives. */
  streamingPhase: ChatPhase | null;
  isSending: boolean;
  isLoadingConversations: boolean;
  isLoadingMessages: boolean;
  error: string | null;
  /** Gmail categories RAG is allowed to search this turn. */
  selectedCategories: EmailCategory[];
  /** True once the store has loaded the persisted preference at least once. */
  categoriesLoaded: boolean;

  fetchConversations: (accountId: string) => Promise<void>;
  /**
   * Point chat at `accountId`, restoring the conversation last open for it.
   *
   * Chat answers from one account at a time, so switching accounts has to
   * switch conversations too — a conversation belongs to the account it was
   * created under. Dropping straight to a new chat each time made switching
   * back and forth lose the thread you were on, so the last conversation used
   * for an account this session is remembered and restored.
   *
   * "This session" is literal: the memory lives in the store, not the DB, so a
   * restart starts clean rather than reopening something from days ago. An
   * account never visited since startup (or whose remembered conversation has
   * since been deleted) opens a fresh chat.
   */
  selectAccount: (accountId: string) => Promise<void>;
  createConversation: (accountId: string, title?: string) => Promise<string>;
  /** Create a chat seeded with the cleaned content of an email thread. */
  createConversationFromThread: (accountId: string, threadId: string) => Promise<string>;
  selectConversation: (id: string | null) => Promise<void>;
  renameConversation: (id: string, title: string) => Promise<void>;
  deleteConversation: (id: string) => Promise<void>;

  /**
   * Send a turn. `contextThreadId` is the thread the main view currently
   * shows (chat panel only) — the backend grounds the answer in it for this
   * turn instead of running retrieval. Omitted by the full-page chat view.
   */
  sendMessage: (content: string, contextThreadId?: string | null, contextAccountId?: string | null) => Promise<void>;
  /** Load persisted categories preference from the DB (called once on mount). */
  loadCategoriesPref: () => Promise<void>;
  /** Update the current selection + persist it so the next session reuses it. */
  setSelectedCategories: (cats: EmailCategory[]) => Promise<void>;

  /** Event handlers — wired once in App.tsx via tauri listen() */
  handleStreamToken: (e: ChatStreamEvent) => void;
  handlePhase: (e: ChatPhaseEvent) => void;
  handleSources: (e: ChatSourcesEvent) => void;
  handleTrace: (e: ChatTraceEvent) => void;
  handleRenamed: (e: ChatRenamedEvent) => void;

  /** Clear everything (called when active account changes) */
  reset: () => void;
}

export const useChatStore = create<ChatStore>((set, get) => ({
  conversations: [],
  activeConversationId: null,
  // Deliberately NOT persisted — "since startup" is the contract.
  lastConversationByAccount: {},
  currentAccountId: null,
  backgroundTurns: {},
  messages: [],
  streamingMessageId: null,
  streamingPhase: null,
  isSending: false,
  isLoadingConversations: false,
  isLoadingMessages: false,
  error: null,
  selectedCategories: [...DEFAULT_CATEGORIES],
  categoriesLoaded: false,

  loadCategoriesPref: async () => {
    try {
      const raw = await api.getPref(CATEGORIES_PREF_KEY);
      set({ selectedCategories: parseCategoriesPref(raw), categoriesLoaded: true });
    } catch {
      // Fall back to default; the dropdown still works, we just didn't read the DB.
      set({ selectedCategories: [...DEFAULT_CATEGORIES], categoriesLoaded: true });
    }
  },

  setSelectedCategories: async (cats) => {
    // Dedupe + enforce canonical order so the backend + persisted value stay predictable.
    const canonical = CHAT_CATEGORY_ORDER.filter((c) => cats.includes(c));
    // Guard: never let the user clear every category — default back to primary.
    const effective = canonical.length > 0 ? canonical : [...DEFAULT_CATEGORIES];
    set({ selectedCategories: effective });
    try {
      await api.setPref(CATEGORIES_PREF_KEY, effective.join(','));
    } catch {
      // Non-fatal — the in-memory selection still applies to the current turn.
    }
  },

  fetchConversations: async (accountId) => {
    set({ isLoadingConversations: true, error: null });
    try {
      const conversations = await api.listChatConversations(accountId);
      set({ conversations, isLoadingConversations: false });
    } catch (e) {
      set({ isLoadingConversations: false, error: errorText(e) });
    }
  },

  selectAccount: async (accountId) => {
    const { currentAccountId, activeConversationId, lastConversationByAccount } = get();
    if (currentAccountId === accountId) return;

    // Remember where we were, so switching back returns to it.
    const remembered = { ...lastConversationByAccount };
    if (currentAccountId && activeConversationId) {
      remembered[currentAccountId] = activeConversationId;
    }
    set({ currentAccountId: accountId, lastConversationByAccount: remembered });

    await get().fetchConversations(accountId);

    // Only restore a conversation that still exists — it may have been deleted
    // since, and selecting a dead id would surface as an error on load.
    const previous = remembered[accountId];
    const stillThere = previous && get().conversations.some((c) => c.id === previous);
    await get().selectConversation(stillThere ? previous : null);
  },

  createConversation: async (accountId, title) => {
    const conv = await api.createChatConversation(accountId, title);
    set((s) => ({
      conversations: [conv, ...s.conversations],
      activeConversationId: conv.id,
      messages: [],
    }));
    return conv.id;
  },

  createConversationFromThread: async (accountId, threadId) => {
    const conv = await api.createChatConversationWithThread(accountId, threadId);
    // Hydrate messages immediately so the system message (the thread context)
    // is available for the UI to render as a context card.
    const messages = await api.getChatMessages(conv.id);
    set((s) => ({
      conversations: [conv, ...s.conversations],
      activeConversationId: conv.id,
      messages,
    }));
    return conv.id;
  },

  selectConversation: async (id) => {
    // Clear streaming flags from a turn left behind in another conversation:
    // its `done` event is dropped by handleStreamToken's conversation guard,
    // so a stale streamingMessageId would make the freshly loaded copy of that
    // message render as still processing.
    if (!id) {
      set({ activeConversationId: null, messages: [], streamingMessageId: null, streamingPhase: null });
      return;
    }
    set({
      activeConversationId: id,
      isLoadingMessages: true,
      messages: [],
      error: null,
      streamingMessageId: null,
      streamingPhase: null,
    });
    try {
      const messages = await api.getChatMessages(id);
      // If the user switched conversations again before this resolved, ignore.
      if (get().activeConversationId !== id) return;

      // Splice back a turn that kept running while this conversation was off
      // screen. The DB copy is authoritative once the turn ends, but stays
      // empty until then — so fall back to what streamed, and re-show the
      // in-flight status so it doesn't read as a finished empty answer.
      const pending = get().backgroundTurns[id];
      if (!pending) {
        set({ messages, isLoadingMessages: false });
        return;
      }
      set((s) => {
        const remaining = { ...s.backgroundTurns };
        delete remaining[id];
        return {
          messages: messages.map((m) =>
            m.id === pending.messageId && !m.content ? { ...m, content: pending.content } : m,
          ),
          isLoadingMessages: false,
          backgroundTurns: remaining,
          streamingMessageId: pending.done ? null : pending.messageId,
          streamingPhase: pending.done ? null : pending.phase,
        };
      });
    } catch (e) {
      if (get().activeConversationId !== id) return;
      set({ isLoadingMessages: false, error: errorText(e) });
    }
  },

  renameConversation: async (id, title) => {
    await api.renameChatConversation(id, title);
    set((s) => ({
      conversations: s.conversations.map((c) => (c.id === id ? { ...c, title } : c)),
    }));
  },

  deleteConversation: async (id) => {
    await api.deleteChatConversation(id);
    set((s) => {
      const remaining = s.conversations.filter((c) => c.id !== id);
      const wasActive = s.activeConversationId === id;
      return {
        conversations: remaining,
        activeConversationId: wasActive ? null : s.activeConversationId,
        messages: wasActive ? [] : s.messages,
      };
    });
  },

  sendMessage: async (content, contextThreadId, contextAccountId) => {
    const trimmed = content.trim();
    if (!trimmed) return;
    const conversationId = get().activeConversationId;
    if (!conversationId) {
      set({ error: 'No active conversation' });
      return;
    }
    if (get().isSending) return;
    // Clear any stale phase from a previous turn up front. We must NOT clear it
    // again when the command returns: a fast turn (e.g. thread-bound chat) can
    // emit its first phase during the await, and that early phase must survive
    // the streamingMessageId assignment so the status shows instead of bare dots.
    set({ isSending: true, error: null, streamingPhase: null });

    try {
      const { userMessage, assistantMessage } = await api.sendChatMessage(
        conversationId,
        trimmed,
        get().selectedCategories,
        contextThreadId,
        contextAccountId,
      );
      // Only mutate if we're still on the same conversation.
      if (get().activeConversationId !== conversationId) return;
      set((s) => ({
        messages: [...s.messages, userMessage, assistantMessage],
        streamingMessageId: assistantMessage.id,
        isSending: false,
      }));
    } catch (e) {
      set({ isSending: false, error: errorText(e), streamingMessageId: null, streamingPhase: null });
    }
  },

  handleStreamToken: (evt) => {
    const { activeConversationId } = get();
    if (evt.conversationId !== activeConversationId) {
      // Not on screen — accumulate so returning to it shows the answer and
      // whether it is still running, rather than an empty bubble.
      set((s) => {
        const prev = s.backgroundTurns[evt.conversationId];
        const base = evt.replace ? '' : (prev?.content ?? '');
        return {
          backgroundTurns: {
            ...s.backgroundTurns,
            [evt.conversationId]: {
              messageId: evt.messageId,
              content: evt.error ?? base + evt.token,
              phase: prev?.phase ?? null,
              done: evt.done ?? false,
            },
          },
        };
      });
      return;
    }

    set((s) => {
      const idx = s.messages.findIndex((m) => m.id === evt.messageId);
      if (idx === -1) return s;
      const existing = s.messages[idx];
      let updated: ChatMessage = evt.error
        ? { ...existing, content: evt.error }
        : // `replace` resets the bubble (contradiction-guard retry overwrites
          // an already-streamed wrong answer); default appends.
          { ...existing, content: evt.replace ? evt.token : existing.content + evt.token };
      // On the final event, persist stats from the backend.
      if (evt.done) {
        updated = {
          ...updated,
          tokenCount: evt.tokenCount ?? updated.tokenCount,
          latencyMs: evt.latencyMs ?? updated.latencyMs,
        };
      }
      const messages = [...s.messages];
      messages[idx] = updated;
      const streamingMessageId = evt.done ? null : s.streamingMessageId;
      // The turn is over once `done` fires — drop the processing status so a
      // stale "Generating…" can't linger under the finished answer.
      const streamingPhase = evt.done ? null : s.streamingPhase;
      const error = evt.error ?? s.error;
      return { messages, streamingMessageId, streamingPhase, error };
    });
  },

  handlePhase: (evt) => {
    const { activeConversationId, streamingMessageId } = get();
    if (evt.conversationId !== activeConversationId) {
      // Keep the status for a turn running off screen (see `backgroundTurns`).
      set((s) => {
        const prev = s.backgroundTurns[evt.conversationId];
        return {
          backgroundTurns: {
            ...s.backgroundTurns,
            [evt.conversationId]: {
              messageId: prev?.messageId ?? evt.messageId,
              content: prev?.content ?? '',
              phase: evt.phase,
              done: prev?.done ?? false,
            },
          },
        };
      });
      return;
    }
    // Scope to the active conversation's in-flight turn so a late event from a
    // previous turn can't flip the status back. The streaming id is only known
    // once sendMessage's command returns, so a turn that reaches its first
    // emit_phase quickly (e.g. thread-bound chat jumping straight to
    // RunningTools) can beat that assignment — accept the event while the id is
    // still null. handleStreamToken nulls streamingPhase on `done`, so no event
    // after the turn ends can re-show a phase.
    if (streamingMessageId !== null && evt.messageId !== streamingMessageId) return;
    set({ streamingPhase: evt.phase });
  },

  handleSources: (evt) => {
    const { activeConversationId } = get();
    if (evt.conversationId !== activeConversationId) return;

    set((s) => {
      const idx = s.messages.findIndex((m) => m.id === evt.messageId);
      if (idx === -1) return s;
      const updated: ChatMessage = { ...s.messages[idx], sources: evt.sources };
      const messages = [...s.messages];
      messages[idx] = updated;
      return { messages };
    });
  },

  handleTrace: (evt) => {
    const { activeConversationId } = get();
    if (evt.conversationId !== activeConversationId) return;

    set((s) => {
      const idx = s.messages.findIndex((m) => m.id === evt.messageId);
      if (idx === -1) return s;
      // Backend ships the email-ref + draft-ref allowlists on the same
      // end-of-turn event as the trace. Splice both onto the message so
      // the bubble's `email://` / `draft://` validators see them the moment
      // streaming concludes.
      const updated: ChatMessage = {
        ...s.messages[idx],
        trace: evt.trace,
        referencedEmailIds: evt.referencedEmailIds ?? [],
        referencedDraftIds: evt.referencedDraftIds ?? [],
      };
      const messages = [...s.messages];
      messages[idx] = updated;
      return { messages };
    });
  },

  handleRenamed: (evt) => {
    // Update both the conversation list and any currently-open conversation
    // header. This event is global (not scoped to the active conversation)
    // so the sidebar can reflect renames even for other open chats.
    set((s) => ({
      conversations: s.conversations.map((c) => (c.id === evt.conversationId ? { ...c, title: evt.title } : c)),
    }));
  },

  reset: () => {
    set({
      conversations: [],
      activeConversationId: null,
      messages: [],
      streamingMessageId: null,
      streamingPhase: null,
      isSending: false,
      isLoadingConversations: false,
      isLoadingMessages: false,
      error: null,
    });
  },
}));
