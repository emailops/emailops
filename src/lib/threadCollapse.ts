import type { Email } from '@/types';

export type ThreadViewItem =
  | { type: 'email'; email: Email; index: number }
  | { type: 'collapsed'; count: number; emailIds: string[] };

/**
 * Given a list of thread emails (oldest first), returns the items to render.
 *
 * When `isExpanded` is false and the thread has 4+ messages, the middle
 * messages are collapsed into a single placeholder item showing the count.
 * The first and last messages are always visible.
 */
export function getThreadViewItems(emails: Email[], isExpanded: boolean): ThreadViewItem[] {
  if (isExpanded || emails.length < 4) {
    return emails.map((email, index) => ({ type: 'email', email, index }));
  }

  const first = emails[0];
  const last = emails[emails.length - 1];
  const middle = emails.slice(1, emails.length - 1);

  return [
    { type: 'email', email: first, index: 0 },
    { type: 'collapsed', count: middle.length, emailIds: middle.map((e) => e.id) },
    { type: 'email', email: last, index: emails.length - 1 },
  ];
}
