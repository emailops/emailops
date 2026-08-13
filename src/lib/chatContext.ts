import type { ViewMode } from '@/components/Sidebar/Sidebar';
import { isEmailListView } from '@/lib/viewNavigation';
import type { EmailTab } from '@/stores/emailStore';
import type { ChatMessage, Email } from '@/types';

/**
 * The thread the chat panel offers as ambient context — i.e. what the user is
 * currently looking at in the main view.
 */
export interface ChatContext {
  threadId: string;
  accountId: string;
  /** Shown on the context chip. May be empty for a no-subject thread. */
  subject: string;
}

export interface ChatContextInput {
  viewMode: ViewMode;
  activeTab: EmailTab | null;
  selectedEmail: Email | null;
}

/**
 * Pure planner: map "what the main view is showing" to the thread the chat
 * panel should ground the next turn in, or `null` for no context (the chat
 * then behaves exactly as it always has — normal retrieval).
 *
 * Precedence mirrors what is actually on screen:
 *   1. An active *thread* tab is the foreground content — it wins.
 *   2. Otherwise the selected email in the split/full-width pane.
 * Compose and attachment tabs are not threads, so they fall through to the
 * selected email rather than blanking the context.
 *
 * Only mailbox-backed views (inbox/sent/spam/deleted/folders) produce context.
 * In calendar, contacts, tasks, memory, lenses or the dashboard there is no
 * thread on screen, even though `selectedEmail` may still be set underneath.
 */
export function deriveChatContext({ viewMode, activeTab, selectedEmail }: ChatContextInput): ChatContext | null {
  if (!isEmailListView(viewMode)) return null;

  if (activeTab?.type === 'thread') {
    return {
      threadId: activeTab.threadId,
      accountId: activeTab.accountId,
      subject: activeTab.subject,
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
  return { threadId: context.threadId, accountId: context.accountId };
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
  return context ? `${context.accountId}:${context.threadId}` : null;
}
