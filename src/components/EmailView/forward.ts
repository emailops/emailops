/**
 * Forwarding: turning a received message into a new one addressed elsewhere.
 *
 * A forward is NOT a reply. It starts with no recipients, it does not thread
 * onto the original conversation, and its body carries the original message
 * with its headers — the point of forwarding is that the person receiving it
 * can see who wrote what, and when.
 *
 * Pure so the shape of the quote can be tested without a webview.
 */

import { htmlToPlainText } from '@/lib/composeHtml';
import type { Email } from '@/types';

/** Prefixes that already mark a subject as forwarded, in the languages the app
 *  ships plus the ones mail clients emit regardless of UI language. */
const FORWARD_PREFIXES = ['fwd:', 'fw:', 'rv:', 'wg:', 'tr:'];

/**
 * `Fwd: <subject>` — but never `Fwd: Fwd: <subject>`.
 *
 * Forwarding a message that was already forwarded to you is the common case,
 * and stacking prefixes is how a subject line turns into noise.
 */
export function forwardSubject(subject: string): string {
  const trimmed = subject.trim();
  if (!trimmed) return 'Fwd:';
  const lower = trimmed.toLowerCase();
  if (FORWARD_PREFIXES.some((p) => lower.startsWith(p))) return trimmed;
  return `Fwd: ${trimmed}`;
}

export interface ForwardLabels {
  header: string;
  from: string;
  date: string;
  subject: string;
  to: string;
  cc: string;
}

/**
 * The quoted original, as plain text: the header block, then the message.
 *
 * Deliberately the conventional header block every mail client writes, because
 * the recipient's client is the one that has to make sense of it — and because
 * the Lens extractor, among others, recognises exactly this shape when it looks
 * inside forwarded mail for the original booking.
 */
export function forwardQuote(email: Email, labels: ForwardLabels, formatDate: (ts: number) => string): string {
  // `sender` is only the display name; the address is what lets the
  // recipient reply to the original author.
  const from = email.sender ? `${email.sender} <${email.senderEmail}>` : email.senderEmail;
  const lines = [
    '',
    '',
    `---------- ${labels.header} ----------`,
    `${labels.from}: ${from}`,
    `${labels.date}: ${formatDate(email.timestamp)}`,
    `${labels.subject}: ${email.subject || ''}`,
    `${labels.to}: ${email.recipients.join(', ')}`,
  ];
  // Only mention Cc when there was one — an empty "Cc:" line reads as a bug.
  if (email.cc.length > 0) lines.push(`${labels.cc}: ${email.cc.join(', ')}`);
  lines.push('');
  const body = htmlToPlainText(email.body).trim();
  if (body) lines.push(body, '');
  return lines.join('\n');
}

/**
 * The body of the message being forwarded.
 *
 * A thread loads without bodies (only the selected message's is preloaded), so
 * the one being forwarded may still need fetching. A failed fetch degrades to
 * headers and attachments only, and `onError` hears why.
 */
export async function loadForwardBody(
  email: Email,
  fetchBody: () => Promise<string>,
  onError: (err: unknown) => void,
): Promise<string> {
  if (email.body) return email.body;
  try {
    return await fetchBody();
  } catch (err) {
    onError(err);
    return '';
  }
}
