import type { ViewMode } from '@/components/Sidebar/Sidebar';
import type { InboxLayout } from '@/types';

export interface ViewChangePlan {
  /** Clear search query, active smart filter, and reset categories to primary. */
  resetInboxFilters: boolean;
  /** Close any open email/tab so the selected view's panel is visible. */
  closeOpenEmail: boolean;
}

/**
 * Pure planner for a sidebar view switch. In full-width layout the email pane
 * replaces the list, so any open email must be closed for the chosen view to show.
 */
export function planViewChange(mode: ViewMode, layout: InboxLayout): ViewChangePlan {
  return {
    resetInboxFilters: mode === 'inbox',
    closeOpenEmail: layout === 'full-width',
  };
}
