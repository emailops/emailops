// How the reader lays out an open conversation (Settings → Appearance →
// Conversation view). Pure; the preference lives in
// `src/stores/conversationViewStore.ts` and `EmailView` renders the plan.

import type { Email } from '@/types';
import { isOwnMessage } from './emailFormatting';

/** `per-message`: the latest message open, the others expanded one by one
 *  (the default). `own-first`: every message open, the user's latest own
 *  message on top. */
export type ConversationView = 'per-message' | 'own-first';

export const CONVERSATION_VIEWS: readonly ConversationView[] = ['per-message', 'own-first'];

/** SQLite preference holding the view. */
export const CONVERSATION_VIEW_PREF = 'ui.conversation_view';

export const DEFAULT_CONVERSATION_VIEW: ConversationView = 'per-message';

export function parseConversationView(raw: string | null): ConversationView {
  return CONVERSATION_VIEWS.find((v) => v === raw) ?? DEFAULT_CONVERSATION_VIEW;
}

export interface OwnFirstPlan {
  /** The user's most recent message in the conversation, shown first; null
   *  when they never wrote in it. */
  own: Email | null;
  /** Every other message, oldest first. */
  rest: Email[];
}

/**
 * The own-message-first layout of one conversation of `accountId`.
 *
 * Only that account's messages, each id once (the first copy). Ordered by
 * date, oldest first; messages with the same date keep the conversation's
 * order. "Own" is the inbox rows' rule (`isOwnMessage`) against the account's
 * address `ownerEmail`: the provider's sent flag, the Sent mailbox, or the
 * account's address outside Inbox and Spam. The latest own message moves to
 * the top and the rest keep that order.
 */
export function planOwnFirst(
  thread: readonly Email[],
  accountId: string,
  ownerEmail: string | undefined,
): OwnFirstPlan {
  const seen = new Set<string>();
  const messages = thread.filter((e) => {
    if (e.accountId !== accountId || seen.has(e.id)) return false;
    seen.add(e.id);
    return true;
  });
  // Array.prototype.sort is stable: equal dates keep the conversation's order.
  messages.sort((a, b) => a.timestamp - b.timestamp);
  const own = [...messages].reverse().find((e) => isOwnMessage(e, ownerEmail)) ?? null;
  return { own, rest: messages.filter((e) => e !== own) };
}
