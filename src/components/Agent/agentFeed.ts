import type { OutgoingMessage } from '@/lib/api';
import { plainTextToHtml, prepareOutgoingHtml } from '@/lib/composeHtml';
import { insertSignature, signatureFor } from '@/lib/signature';
import type { AccountSignature, AgentAction, AgentRun, Draft } from '@/types';

/** The feed as a chat reads it: oldest first, newest at the bottom. */
export function chronological(feed: AgentRun[]): AgentRun[] {
  return [...feed].sort((a, b) => a.createdAt - b.createdAt);
}

/** The side panel's two lists: what waits for the user (actions to approve,
 *  drafts to send or discard), and the rest. */
export function splitActions(actions: AgentAction[]): { pending: AgentAction[]; recent: AgentAction[] } {
  const waiting = (a: AgentAction) => a.status === 'pending' || a.needsReview;
  return {
    pending: actions.filter(waiting),
    recent: actions.filter((a) => !waiting(a)),
  };
}

interface ReplyInput {
  accountId: string;
  /** The email the draft answers. */
  emailId: string;
  draft: Pick<Draft, 'toAddresses' | 'ccAddresses'>;
  /** The draft text as the user left it. */
  text: string;
  signature: AccountSignature | null;
}

/** The reply to send from the review pane: the reviewed text plus the
 *  account's reply signature, like a composer would send it. */
export function buildReplyMessage({ accountId, emailId, draft, text, signature }: ReplyInput): OutgoingMessage {
  const html = insertSignature(plainTextToHtml(text), signatureFor(signature, 'reply'), 'reply');
  const prepared = prepareOutgoingHtml(html);
  return {
    accountId,
    replyToEmailId: emailId,
    to: draft.toAddresses,
    cc: draft.ccAddresses,
    // A reply takes its parent's subject ("Re: …") backend-side.
    subject: '',
    body: prepared.plainText,
    bodyHtml: prepared.bodyHtml,
    inlineImages: prepared.inlineImages,
    attachments: [],
  };
}
