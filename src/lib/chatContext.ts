import type { ViewMode } from '@/components/Sidebar/Sidebar';
import { isEmailListView } from '@/lib/viewNavigation';
import type { EmailTab } from '@/stores/emailStore';
import type { Attachment, ChatMessage, Email } from '@/types';

/**
 * What the chat panel offers as ambient context — i.e. what the user is
 * currently looking at in the main view: a thread, or a PDF attachment.
 */
export interface ChatContext {
  threadId: string;
  accountId: string;
  /** Shown on the context chip: the thread's subject (may be empty), or the
   *  PDF's file name. */
  subject: string;
  /** Set when the context is an open PDF tab: its attachment id. The backend
   *  then grounds the turn in the PDF's text instead of the thread. */
  attachmentId?: string;
}

/** A PDF by its mime type, or by its name when the sender declared a generic
 *  type. Mirrors the backend's `open_document::is_pdf`. */
export function isPdfAttachment(mimeType: string, filename: string): boolean {
  return mimeType.toLowerCase() === 'application/pdf' || filename.toLowerCase().endsWith('.pdf');
}

export interface ChatContextInput {
  viewMode: ViewMode;
  activeTab: EmailTab | null;
  selectedEmail: Email | null;
  /** The file selected in the Attachments view (one an attachment rule saved). */
  selectedAttachment?: Attachment | null;
}

/**
 * Pure planner: map "what the main view is showing" to the thread the chat
 * panel should ground the next turn in, or `null` for no context (the chat
 * then behaves exactly as it always has — normal retrieval).
 *
 * Precedence mirrors what is actually on screen:
 *   1. An active *thread* tab is the foreground content — it wins.
 *   2. An active *PDF* tab: the chat reads the document (its thread id, when
 *      an email is selected, is kept only so the chip key stays stable).
 *   3. Otherwise the selected email in the split/full-width pane.
 * Compose tabs and non-PDF attachment tabs fall through to the selected email
 * rather than blanking the context.
 *
 * Only mailbox-backed views (inbox/sent/spam/deleted/folders) produce thread
 * context. The Attachments view offers the PDF selected in its viewer. In
 * calendar, contacts, tasks, memory, lenses or the dashboard there is nothing
 * on screen to read, even though `selectedEmail` may still be set underneath.
 */
export function deriveChatContext({
  viewMode,
  activeTab,
  selectedEmail,
  selectedAttachment,
}: ChatContextInput): ChatContext | null {
  if (viewMode === 'attachments') {
    if (!selectedAttachment || !isPdfAttachment(selectedAttachment.mimeType, selectedAttachment.filename)) return null;
    return {
      threadId: '',
      accountId: selectedAttachment.accountId,
      subject: selectedAttachment.filename,
      attachmentId: selectedAttachment.id,
    };
  }
  if (!isEmailListView(viewMode)) return null;

  if (activeTab?.type === 'thread') {
    return {
      threadId: activeTab.threadId,
      accountId: activeTab.accountId,
      subject: activeTab.subject,
    };
  }

  if (activeTab?.type === 'attachment' && isPdfAttachment(activeTab.mimeType, activeTab.filename)) {
    return {
      threadId: selectedEmail?.threadId ?? '',
      accountId: activeTab.accountId,
      subject: activeTab.filename,
      attachmentId: activeTab.id,
    };
  }

  if (selectedEmail) {
    return {
      threadId: selectedEmail.threadId,
      accountId: selectedEmail.accountId,
      subject: selectedEmail.subject,
    };
  }

  return null;
}

/** What a turn sends as ambient grounding, or `null` for plain retrieval. */
export interface ChatTurnContext {
  threadId: string;
  accountId: string;
  /** The open PDF, when that is the context. */
  attachmentId?: string;
}

/**
 * Pure planner: what the panel sends with a turn, given the offered context and
 * whether its chip is armed.
 *
 * The `accountId` half is the point. It is the *thread's* account, which is not
 * necessarily the account the chat runs on — in unified ("All accounts") mode
 * the panel runs on the first enabled account while the open thread can belong
 * to any of them. Sending only the thread id made the backend look it up under
 * the chat's account, find nothing, and silently answer from retrieval instead
 * ("resume el correo" → "which email do you mean?" with the email open).
 */
export function chatTurnContext(context: ChatContext | null, active: boolean): ChatTurnContext | null {
  if (!active || !context) return null;
  const turn: ChatTurnContext = { threadId: context.threadId, accountId: context.accountId };
  if (context.attachmentId) turn.attachmentId = context.attachmentId;
  return turn;
}

/** What the panel should do about the thread the main view has open. */
export type ChatContextOffer =
  /** Nothing to show — no thread open, or the conversation owns its grounding. */
  | { kind: 'none' }
  /** Usable now: the thread belongs to the account chat answers from. */
  | { kind: 'offered'; context: ChatContext }
  /** Open, but owned by a different account than chat is scoped to. */
  | { kind: 'otherAccount'; context: ChatContext };

/**
 * Decide how the open thread relates to the chat's account.
 *
 * Three things can independently disagree in "All accounts" mode: what the
 * mail list shows, which account chat answers from, and which account the open
 * email belongs to. A thread from an account chat cannot read must not be
 * grounded on — one turn from a mailbox every following turn cannot search is
 * incoherent.
 *
 * But dropping it *silently* is its own trap: asking about the email plainly on
 * screen then gets an answer from a different mailbox, with nothing saying why.
 * So the mismatch is a state of its own, and the panel says which account the
 * email is in and offers to switch.
 */
export function planChatContextOffer(
  context: ChatContext | null,
  chatAccountId: string | null,
  conversationIsThreadBound: boolean,
): ChatContextOffer {
  // A conversation seeded via "Chat about this thread" already owns its
  // grounding; the backend ignores ambient context for it, so neither offering
  // nor warning would describe the next answer truthfully.
  if (conversationIsThreadBound) return { kind: 'none' };
  if (!context || !chatAccountId) return { kind: 'none' };
  return context.accountId === chatAccountId ? { kind: 'offered', context } : { kind: 'otherAccount', context };
}

/**
 * Whether this conversation was seeded with a thread at creation ("Chat about
 * this thread"), which the backend's `plan_turn_mode` gives precedence over
 * ambient view context.
 *
 * The panel uses this to suppress the context chip: offering a thread the
 * backend is going to ignore would tell the user their next answer comes from
 * the open thread when it actually comes from the seeded one.
 */
export function isConversationThreadBound(messages: ChatMessage[]): boolean {
  return messages.some((m) => m.role === 'system');
}

/**
 * Stable identity for a context, so the panel can tell "the user moved to a
 * different thread" (drop any dismissal, offer the new context) apart from a
 * re-render of the same one.
 */
export function chatContextKey(context: ChatContext | null): string | null {
  if (!context) return null;
  const base = `${context.accountId}:${context.threadId}`;
  return context.attachmentId ? `${base}:${context.attachmentId}` : base;
}
