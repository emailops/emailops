import type { ChatMessage } from '@/types';

/** A `[n]` citation marker (same rule as `MarkdownContent`) or an `email://ID` link. */
const EMAIL_REFERENCE = /\[(\d+)\](?!\()|email:\/\/([^\s)\]>]+)/g;

/** The emails an assistant answer actually cites: sources behind its `[n]`
 *  markers, plus `email://` links that pass the turn's tool allowlist (the same
 *  guard `MarkdownContent` applies). Deduplicated, in order of appearance.
 *  Retrieved-but-uncited sources and the rest of `referencedEmailIds` are left
 *  out — those are what the tools returned, not what the answer points at.
 *  An answer that cites nothing falls back to its sources, in citation order:
 *  the backend already narrowed them to what the turn rested on. */
export function collectReferencedEmailIds(
  message: Pick<ChatMessage, 'content' | 'sources' | 'referencedEmailIds'>,
): string[] {
  const sourceByNumber = new Map(message.sources.map((source) => [source.citationNumber, source.emailId]));
  const allowlist = new Set(message.referencedEmailIds ?? []);
  const ids = new Set<string>();
  for (const [, citation, linkedId] of message.content.matchAll(EMAIL_REFERENCE)) {
    const id = citation ? sourceByNumber.get(Number(citation)) : allowlist.has(linkedId) ? linkedId : undefined;
    if (id) ids.add(id);
  }
  if (ids.size === 0) {
    return [...message.sources].sort((a, b) => a.citationNumber - b.citationNumber).map((source) => source.emailId);
  }
  return [...ids];
}

/** Search-box query selecting exactly these emails (the backend `id:` operator). */
export function buildIdSearchQuery(emailIds: string[]): string {
  return emailIds.map((id) => `id:${id}`).join(' ');
}
