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
  ChatResearchProgressEvent,
  ChatSourcesEvent,
  ChatStreamEvent,
  ChatTraceEvent,
  EmailCategory,
  ResearchEstimate,
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
  backgroundTurns: Record<string, BackgroundTurn>;
  /** Account key the chat was last reset for (see `resetForAccount`). */
  resetAccountKey: string | null;
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
  /** Research mode armed for the NEXT message only: the send disarms it, so a
   *  follow-up question is a normal (fast) turn unless the user arms it again. */
  researchMode: boolean;
  /** Batch progress of the in-flight research turn; null otherwise. */
  researchProgress: ChatResearchProgressEvent | null;
  /** The research reading right now in ANY conversation — for the status bar
   *  and the quit confirmation, which do not care which chat is on screen. */
  runningResearch: ChatResearchProgressEvent | null;
  /** The user tried to quit while research runs; the confirmation is open. */
  researchExitRequested: boolean;
  /** When the in-flight research started reading (ms), for the time left. */
  researchStartedAt: number | null;
  /** The user pressed Stop; the run is finishing its batch and the report. */
  researchStopping: boolean;
  /** A research question awaiting the user's go-ahead: its estimate (how many
   *  emails, how long) is shown before anything is sent. */
  pendingResearch: PendingResearch | null;
  /** Text to put back in the input (a cancelled research question); the
   *  nonce lets the same text be restored twice. */
  inputPrefill: { text: string; nonce: number } | null;
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
  prefillInput: (text: string) => void;
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
  sendMessage: (
    content: string,
    contextThreadId?: string | null,
    contextAccountId?: string | null,
    contextView?: api.ChatViewContext | null,
  ) => Promise<void>;
  /**
   * "This answer is wrong" → a fresh corrective turn.
   *
   * The rejected answer stays in the history, marked, and the correction runs
   * as a new turn carrying what the user said was wrong. Nothing is persisted
   * beyond the conversation itself.
   */
  retryWithCorrection: (
    rejectedMessageId: string,
    reason: string,
    contextThreadId?: string | null,
    contextAccountId?: string | null,
    contextView?: api.ChatViewContext | null,
  ) => Promise<void>;
  /** Ids of assistant messages the user marked wrong, for this session. */
  rejectedMessageIds: string[];
  /** Shared turn dispatcher behind `sendMessage` and `retryWithCorrection` —
   *  one place that owns the isSending guard, the optimistic append and the
   *  error handling, so a correction cannot drift from a normal turn. */
  dispatchTurn: (content: string, opts: TurnOptions) => Promise<void>;
  /** Send the pending research question the user confirmed. */
  confirmResearch: () => Promise<void>;
  /** Drop the pending research question and hand its text back to the input. */
  cancelResearch: () => void;
  /** Stop the running research: it writes its report from what it has read. */
  stopResearch: () => Promise<void>;
  /** Load persisted categories preference from the DB (called once on mount). */
  loadCategoriesPref: () => Promise<void>;
  /** Update the current selection + persist it so the next session reuses it. */
  setSelectedCategories: (cats: EmailCategory[]) => Promise<void>;

  /** Event handlers — wired once in App.tsx via tauri listen() */
  handleStreamToken: (e: ChatStreamEvent) => void;
  handlePhase: (e: ChatPhaseEvent) => void;
  handleResearchProgress: (e: ChatResearchProgressEvent) => void;
  /** The backend held a quit because research runs: ask the user. */
  handleResearchExitRequested: () => void;
  dismissResearchExit: () => void;
  setResearchMode: (on: boolean) => void;
  handleSources: (e: ChatSourcesEvent) => void;
  handleTrace: (e: ChatTraceEvent) => void;
  handleRenamed: (e: ChatRenamedEvent) => void;

  /** Clear everything. */
  reset: () => void;
  /**
   * Clear the chat because the app switched to `accountKey` — a no-op when it
   * was already reset for that key. The App effect that calls this re-runs
   * whenever the account list reloads, not only on a real switch, and an
   * unconditional reset there emptied the conversation list (and dropped an
   * in-flight turn) while the account stayed the same.
   */
  resetForAccount: (accountKey: string) => void;
}

/** What a turn carries besides its text. */
interface TurnOptions {
  contextThreadId?: string | null;
  contextAccountId?: string | null;
  contextView?: api.ChatViewContext | null;
  correction?: api.ChatCorrection | null;
  research?: boolean;
  researchEstimateId?: string | null;
}

/** A research question waiting for the user to confirm its estimate. */
export interface PendingResearch {
  content: string;
  opts: TurnOptions;
  status: 'estimating' | 'ready' | 'error';
  estimate: ResearchEstimate | null;
  error: string | null;
}

/** A turn still running in a conversation that is not on screen. */
interface BackgroundTurn {
  messageId: string;
  content: string;
  phase: ChatPhase | null;
  /** Research batch progress, so a research turn shows where it is on return. */
  research: ChatResearchProgressEvent | null;
  done: boolean;
}

export const useChatStore = create<ChatStore>((set, get) => {
  /** Estimate a research question and hold it for the user to confirm. */
  const holdForEstimate = async (content: string, opts: TurnOptions) => {
    const trimmed = content.trim();
    const conversationId = get().activeConversationId;
    if (!trimmed || !conversationId) return;
    set({ pendingResearch: { content: trimmed, opts, status: 'estimating', estimate: null, error: null } });
    try {
      const estimate = await api.estimateResearch(
        conversationId,
        trimmed,
        get().selectedCategories,
        opts.correction ?? null,
      );
      if (get().pendingResearch?.content !== trimmed) return; // cancelled meanwhile
      set({ pendingResearch: { content: trimmed, opts, status: 'ready', estimate, error: null } });
    } catch (e) {
      if (get().pendingResearch?.content !== trimmed) return;
      set({ pendingResearch: { content: trimmed, opts, status: 'error', estimate: null, error: errorText(e) } });
    }
  };

  return {
    conversations: [],
    activeConversationId: null,
    // Deliberately NOT persisted — "since startup" is the contract.
    lastConversationByAccount: {},
    currentAccountId: null,
    backgroundTurns: {},
    resetAccountKey: null,
    messages: [],
    streamingMessageId: null,
    streamingPhase: null,
    researchMode: false,
    researchProgress: null,
    researchStartedAt: null,
    researchStopping: false,
    runningResearch: null,
    researchExitRequested: false,
    pendingResearch: null,
    inputPrefill: null,
    isSending: false,
    isLoadingConversations: false,
    isLoadingMessages: false,
    error: null,
    rejectedMessageIds: [],
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

      // A conversation of this account with a turn still running (a research
      // that reads for minutes) wins: coming back to the account means coming
      // back to it. Otherwise restore the last one — only if it still exists, as
      // it may have been deleted since and a dead id would error on load.
      const inAccount = (id: string) => get().conversations.some((c) => c.id === id);
      const running = Object.entries(get().backgroundTurns).find(([id, turn]) => !turn.done && inAccount(id))?.[0];
      const researching = get().runningResearch?.conversationId;
      const previous = remembered[accountId];
      const target =
        running ??
        (researching && inAccount(researching) ? researching : undefined) ??
        (previous && inAccount(previous) ? previous : null);
      await get().selectConversation(target);
    },

    prefillInput: (text) => set((s) => ({ inputPrefill: { text, nonce: (s.inputPrefill?.nonce ?? 0) + 1 } })),

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
      //
      // A turn still running in the conversation being left is parked in
      // `backgroundTurns` first. Its next event can be ~20 s away (a research
      // batch), and returning before it arrived found no record and showed an
      // empty, finished-looking answer. Re-selecting the open conversation
      // parks and restores it the same way.
      const parked = parkRunningTurn(get());
      if (!id) {
        set({
          activeConversationId: null,
          messages: [],
          streamingMessageId: null,
          streamingPhase: null,
          researchProgress: null,
          backgroundTurns: parked,
        });
        return;
      }
      set({
        activeConversationId: id,
        isLoadingMessages: true,
        messages: [],
        error: null,
        streamingMessageId: null,
        streamingPhase: null,
        researchProgress: null,
        backgroundTurns: parked,
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
            researchProgress: pending.done ? null : pending.research,
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

    sendMessage: async (content, contextThreadId, contextAccountId, contextView) => {
      const opts: TurnOptions = { contextThreadId, contextAccountId, contextView };
      if (!get().researchMode) {
        await get().dispatchTurn(content, opts);
        return;
      }
      // Research: estimate first, send only once the user confirms. The toggle
      // is spent on this question either way.
      set({ researchMode: false });
      await holdForEstimate(content, opts);
    },

    confirmResearch: async () => {
      const pending = get().pendingResearch;
      if (pending?.status !== 'ready' || !pending.estimate) return;
      set({ pendingResearch: null });
      await get().dispatchTurn(pending.content, {
        ...pending.opts,
        research: true,
        researchEstimateId: pending.estimate.estimateId,
      });
    },

    cancelResearch: () => {
      const pending = get().pendingResearch;
      if (!pending) return;
      set((s) => ({
        pendingResearch: null,
        inputPrefill: { text: pending.content, nonce: (s.inputPrefill?.nonce ?? 0) + 1 },
      }));
    },

    stopResearch: async () => {
      const messageId = get().streamingMessageId;
      if (!messageId) return;
      set({ researchStopping: true });
      try {
        await api.stopResearch(messageId);
      } catch (e) {
        set({ researchStopping: false, error: errorText(e) });
      }
    },

    retryWithCorrection: async (rejectedMessageId, reason, contextThreadId, contextAccountId, contextView) => {
      const trimmed = reason.trim();
      if (!trimmed) return;
      // Mark first so the rejected bubble is struck through while the corrective
      // turn runs, rather than only once the new answer lands.
      set((s) => ({
        rejectedMessageIds: s.rejectedMessageIds.includes(rejectedMessageId)
          ? s.rejectedMessageIds
          : [...s.rejectedMessageIds, rejectedMessageId],
      }));
      const opts: TurnOptions = {
        contextThreadId,
        contextAccountId,
        contextView,
        correction: { rejectedMessageId, reason: trimmed },
      };
      // A rejected research is retried as a research: the new run reads again
      // with the correction, and — being as long — is estimated and confirmed
      // first, like the original.
      const rejected = get().messages.find((m) => m.id === rejectedMessageId);
      if (rejected?.trace?.research) {
        await holdForEstimate(trimmed, opts);
        return;
      }
      await get().dispatchTurn(trimmed, opts);
    },

    dispatchTurn: async (content, opts) => {
      const { contextThreadId, contextAccountId, contextView, correction } = opts;
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
      const research = opts.research ?? false;
      set({
        isSending: true,
        error: null,
        streamingPhase: null,
        researchProgress: null,
        researchStartedAt: null,
        researchStopping: false,
      });

      try {
        const { userMessage, assistantMessage } = await api.sendChatMessage(
          conversationId,
          trimmed,
          get().selectedCategories,
          contextThreadId,
          contextAccountId,
          contextView,
          correction,
          research,
          opts.researchEstimateId,
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
      // The research that answers this message is over, on screen or not.
      if (evt.done && get().runningResearch?.messageId === evt.messageId) {
        set({ runningResearch: null });
      }
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
                research: prev?.research ?? null,
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
        const researchProgress = evt.done ? null : s.researchProgress;
        const error = evt.error ?? s.error;
        return {
          messages,
          streamingMessageId,
          streamingPhase,
          researchProgress,
          researchStartedAt: evt.done ? null : s.researchStartedAt,
          researchStopping: evt.done ? false : s.researchStopping,
          error,
        };
      });
    },

    handleResearchExitRequested: () => set({ researchExitRequested: true }),
    dismissResearchExit: () => set({ researchExitRequested: false }),

    handleResearchProgress: (evt) => {
      set({ runningResearch: evt });
      const { activeConversationId, streamingMessageId } = get();
      if (evt.conversationId !== activeConversationId) {
        // Off screen: keep it with the parked turn so the return shows it.
        set((s) => {
          const prev = s.backgroundTurns[evt.conversationId];
          if (prev?.done) return s;
          return {
            backgroundTurns: {
              ...s.backgroundTurns,
              [evt.conversationId]: {
                messageId: prev?.messageId ?? evt.messageId,
                content: prev?.content ?? '',
                phase: prev?.phase ?? 'researching',
                research: evt,
                done: false,
              },
            },
          };
        });
        return;
      }
      // Same scoping as `handlePhase`: accept while the id is still unknown.
      if (streamingMessageId !== null && evt.messageId !== streamingMessageId) return;
      // The clock for "time left" starts when reading starts.
      const startedAt = get().researchStartedAt ?? (evt.stage === 'reading' ? Date.now() : null);
      set({ researchProgress: evt, researchStartedAt: startedAt });
    },

    setResearchMode: (on) => set({ researchMode: on }),

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
                research: prev?.research ?? null,
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

    resetForAccount: (accountKey) => {
      if (get().resetAccountKey === accountKey) return;
      get().reset();
      set({ resetAccountKey: accountKey });
    },

    reset: () => {
      set({
        // A turn still running (a research can read for an hour) keeps going on
        // the backend: park it so coming back to its conversation shows it.
        backgroundTurns: parkRunningTurn(get()),
        // Forget the account too, so the chat surfaces' `selectAccount` reloads
        // the list instead of treating the account as already loaded.
        currentAccountId: null,
        conversations: [],
        activeConversationId: null,
        messages: [],
        streamingMessageId: null,
        streamingPhase: null,
        researchMode: false,
        researchProgress: null,
        isSending: false,
        isLoadingConversations: false,
        isLoadingMessages: false,
        error: null,
        rejectedMessageIds: [],
      });
    },
  };
});

/** `backgroundTurns` with the open conversation's running turn parked in it. */
function parkRunningTurn(s: ChatStore): Record<string, BackgroundTurn> {
  const { activeConversationId, streamingMessageId } = s;
  if (!activeConversationId || !streamingMessageId) return s.backgroundTurns;
  const message = s.messages.find((m) => m.id === streamingMessageId);
  return {
    ...s.backgroundTurns,
    [activeConversationId]: {
      messageId: streamingMessageId,
      content: message?.content ?? '',
      phase: s.streamingPhase,
      research: s.researchProgress,
      done: false,
    },
  };
}
