import { htmlToPlainText } from '@/lib/composeHtml';
import { withoutSignature } from '@/lib/signature';
import type { ChatDraftContext, DraftKind } from '@/types';

/**
 * The draft the user is writing, as read-only context for the chat panel.
 * Pure helpers: the composers build the snapshot, the panel decides what a
 * turn sends.
 */

/** The text the user wrote: the body without the EmailOps signature node. A
 *  body without that node (a draft reopened from the provider) is read whole,
 *  with whatever signature or quote it carries. */
export function userTextOf(bodyHtml: string): string {
  return htmlToPlainText(withoutSignature(bodyHtml)).trim();
}

export interface ComposerState {
  kind: DraftKind;
  subject: string;
  to: string[];
  cc: string[];
  bodyHtml: string;
  replyToEmailId: string | null;
}

/** A composer's draft as chat context, or `null` while it holds no text of the
 *  user (only the signature, or nothing). */
export function draftContextOf(state: ComposerState): ChatDraftContext | null {
  const body = userTextOf(state.bodyHtml);
  if (!body) return null;
  return {
    kind: state.kind,
    subject: state.subject,
    to: state.to,
    cc: state.cc,
    body,
    replyToEmailId: state.replyToEmailId,
  };
}

export interface DraftTurnInput {
  snapshot: ChatDraftContext | null;
  /** The account the composer's thread lives in. */
  accountId: string;
  chatAccountId: string | null;
  /** The user switched the draft chip off. */
  disabled: boolean;
  research: boolean;
}

/**
 * What a turn sends as the draft. Nothing while the chip is off or on a
 * research turn (research reads the mailbox, not what is on screen). A reply
 * from another account than the chat's travels without its thread, the same
 * `otherAccount` rule the thread chip follows.
 */
export function draftForTurn({
  snapshot,
  accountId,
  chatAccountId,
  disabled,
  research,
}: DraftTurnInput): ChatDraftContext | null {
  if (!snapshot || disabled || research) return null;
  if (chatAccountId !== null && accountId !== chatAccountId) return { ...snapshot, replyToEmailId: null };
  return snapshot;
}

export type DraftChipScope = 'threadAndReply' | 'reply' | 'newEmail';

/** What the draft chip says travels with the turn. */
export function draftChipScope(kind: DraftKind, threadTravels: boolean): DraftChipScope {
  if (kind === 'new') return 'newEmail';
  return threadTravels ? 'threadAndReply' : 'reply';
}

export interface DraftOfferInput {
  /** The composer on screen, or `null`. */
  kind: DraftKind | null;
  hasUserText: boolean;
  /** The user switched this composer's chip off. */
  disabled: boolean;
  /** The composer's thread is in the chat's account. */
  sameAccount: boolean;
}

export interface DraftOffer {
  /** The chip shows: a composer holding text of the user. */
  shown: boolean;
  /** Its draft travels with the next turn. */
  active: boolean;
  scope: DraftChipScope;
  /** An active reply brings its own thread, so the open thread is not sent
   *  apart and its chip gives way to the draft's. */
  carriesThread: boolean;
}

/** What the chat panel offers of the draft on screen. Pure. */
export function planDraftOffer({ kind, hasUserText, disabled, sameAccount }: DraftOfferInput): DraftOffer {
  const shown = kind !== null && hasUserText;
  const active = shown && !disabled;
  const isReply = kind !== null && kind !== 'new';
  return {
    shown,
    active,
    scope: draftChipScope(kind ?? 'new', isReply && sameAccount),
    carriesThread: active && isReply,
  };
}
