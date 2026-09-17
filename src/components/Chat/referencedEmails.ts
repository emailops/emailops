import type { ChatMessage } from '@/types';

const EMAIL_LINK = /email:\/\/([^\s)\]>]+)/g;

/** The emails an assistant answer points at: its sources, plus the `email://`
 *  links in the text that pass the turn's tool allowlist (the same guard
 *  `MarkdownContent` applies). Deduplicated, in order of appearance. Not every
 *  `referencedEmailIds` entry — that list holds everything the tools returned,
 *  cited or not. */
export function collectReferencedEmailIds(
  message: Pick<ChatMessage, 'content' | 'sources' | 'referencedEmailIds'>,
): string[] {
  const ids = new Set(message.sources.map((source) => source.emailId));
  const allowlist = new Set(message.referencedEmailIds ?? []);
  for (const match of message.content.matchAll(EMAIL_LINK)) {
    if (allowlist.has(match[1])) ids.add(match[1]);
  }
  return [...ids];
}

/** Search-box query selecting exactly these emails (the backend `id:` operator). */
export function buildIdSearchQuery(emailIds: string[]): string {
  return emailIds.map((id) => `id:${id}`).join(' ');
}
