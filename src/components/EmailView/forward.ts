/**
 * Forwarding: turning a received message into a new one addressed elsewhere.
 *
 * A forward is NOT a reply. It starts with no recipients, it does not thread
 * onto the original conversation, and its body carries the original messages
 * with their headers — the point of forwarding is that the person receiving it
 * can see who wrote what, and when.
 *
 * Pure so the shape of the quote can be tested without a webview.
 */

import type { Email, ThreadMessageText } from '@/types';

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

/** One message of the forwarded thread and the text it adds to it. */
export interface ForwardedMessage {
  email: Email;
  text: string;
}

/**
 * The quoted conversation, as plain text: every message of the thread, oldest
 * first, each as a header block followed by what it says.
 *
 * The whole thread, not its latest message: the latest carries only the
 * history its own client chose to quote, and a reply that quotes nothing cuts
 * every message before it out of the forward. Each message's text is its new
 * content (see `loadForwardTexts`), so the conversation reads once.
 *
 * Deliberately the conventional header block every mail client writes, because
 * the recipient's client is the one that has to make sense of it — and because
 * the Lens extractor, among others, recognises exactly this shape when it looks
 * inside forwarded mail for the original booking.
 */
export function forwardQuote(
  messages: ForwardedMessage[],
  labels: ForwardLabels,
  formatDate: (ts: number) => string,
): string {
  const lines = [''];
  for (const { email, text } of messages) {
    // `sender` is only the display name; the address is what lets the
    // recipient reply to the original author.
    const from = email.sender ? `${email.sender} <${email.senderEmail}>` : email.senderEmail;
    lines.push(
      '',
      `---------- ${labels.header} ----------`,
      `${labels.from}: ${from}`,
      `${labels.date}: ${formatDate(email.timestamp)}`,
      `${labels.subject}: ${email.subject || ''}`,
      `${labels.to}: ${email.recipients.join(', ')}`,
    );
    // Only mention Cc when there was one — an empty "Cc:" line reads as a bug.
    if (email.cc.length > 0) lines.push(`${labels.cc}: ${email.cc.join(', ')}`);
    lines.push('');
    const body = text.trim();
    if (body) lines.push(body, '');
  }
  return lines.join('\n');
}

/**
 * Each message of the thread paired with the text it adds to it.
 *
 * The thread loads without bodies, and a reply's body repeats the messages it
 * quotes, so the texts come from the backend, which reads the thread as a
 * unit and drops history an earlier message already holds. A message the
 * backend did not return (the thread changed meanwhile) keeps its headers
 * only; a failed fetch degrades every message to headers, and `onError`
 * hears why.
 */
export async function loadForwardTexts(
  thread: Email[],
  fetchTexts: () => Promise<ThreadMessageText[]>,
  onError: (err: unknown) => void,
): Promise<ForwardedMessage[]> {
  let texts = new Map<string, string>();
  try {
    texts = new Map((await fetchTexts()).map((t) => [t.emailId, t.text]));
  } catch (err) {
    onError(err);
  }
  return thread.map((email) => ({ email, text: texts.get(email.id) ?? '' }));
}
