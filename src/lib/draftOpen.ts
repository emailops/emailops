import * as api from '@/lib/api';
import type { Draft } from '@/types';

/**
 * The draft to open in a compose tab, re-read after asking the provider for
 * changes so a draft edited in Gmail opens with Gmail's content rather than
 * the row the list happened to render.
 *
 * Falls back to the passed draft whenever the provider is unreachable or the
 * row has since disappeared upstream — opening a slightly stale draft always
 * beats refusing to open it. The provider call is throttled backend-side, so
 * clicking a draft right after the Drafts screen loaded costs no network.
 */
export async function freshDraftToOpen(draft: Draft): Promise<Draft> {
  try {
    await api.refreshDrafts(draft.accountId);
    return (await api.getDraft(draft.id)) ?? draft;
  } catch {
    return draft;
  }
}
