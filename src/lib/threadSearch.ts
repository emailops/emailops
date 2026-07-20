import type { Email } from '@/types';

// Approximate the text the user actually sees in the rendered body: drop
// style/script blocks whole (their contents never render), replace remaining
// tags with a space (so text split across elements doesn't fuse into one
// token), and decode the entities that commonly appear in email HTML. This
// keeps matches aligned with what the in-frame highlighter can mark — a hit
// inside a tag attribute would report a match with nothing highlighted.
function htmlToSearchText(html: string): string {
  return html
    .replace(/<(style|script)\b[^>]*>[\s\S]*?<\/\1>/gi, ' ')
    .replace(/<[^>]+>/g, ' ')
    .replace(/&nbsp;/gi, ' ')
    .replace(/&amp;/gi, '&')
    .replace(/&lt;/gi, '<')
    .replace(/&gt;/gi, '>')
    .replace(/&quot;/gi, '"')
    .replace(/&#0?39;/g, "'")
    .replace(/\s+/g, ' ');
}

/**
 * IDs (in thread order) of the emails whose visible content matches the
 * query, case-insensitively. Searches subject, snippet, sender name/address,
 * and the body's rendered text.
 */
export function getThreadSearchMatches(emails: Email[], query: string): string[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  return emails
    .filter((e) =>
      [e.subject, e.snippet, e.sender, e.senderEmail, htmlToSearchText(e.body)].some((field) =>
        field.toLowerCase().includes(q),
      ),
    )
    .map((e) => e.id);
}

export interface OccurrenceSlot {
  emailId: string;
  indexInEmail: number;
}

/**
 * Flatten per-email occurrence counts into a navigable list of occurrence
 * slots, in thread order. Counts arrive asynchronously from each rendered
 * body frame; an email whose count is still unknown — or that matched only
 * via subject/sender (count 0) — contributes a single slot so it stays
 * reachable in the prev/next cycle.
 */
export function buildOccurrenceSlots(matchIds: string[], counts: Record<string, number | undefined>): OccurrenceSlot[] {
  return matchIds.flatMap((emailId) => {
    const n = Math.max(1, counts[emailId] ?? 1);
    return Array.from({ length: n }, (_, indexInEmail) => ({ emailId, indexInEmail }));
  });
}

/** Cyclic prev/next navigation over the match list. -1 when there are no matches. */
export function stepMatchIndex(current: number, delta: number, total: number): number {
  if (total <= 0) return -1;
  return (((current + delta) % total) + total) % total;
}
