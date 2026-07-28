import type { ViewMode } from '@/components/Sidebar/Sidebar';
import type { InboxLayout } from '@/types';

export interface ViewChangePlan {
  /** Clear search query, active smart filter, and reset categories to primary. */
  resetInboxFilters: boolean;
  /** Close any open email/tab so the selected view's panel is visible. */
  closeOpenEmail: boolean;
}

/**
 * Views backed by a mailbox — these all render the same email list, just
 * scoped to inbox/sent/spam/deleted/a custom folder.
 */
export function isEmailListView(mode: ViewMode): boolean {
  return mode === 'inbox' || mode === 'sent' || mode === 'spam' || mode === 'deleted' || mode.startsWith('folder:');
}

/**
 * Pure planner for a sidebar view switch. In full-width layout the email pane
 * replaces the list, so any open email must be closed for the chosen view to show.
 *
 * Filters reset for every mailbox-backed view, not just the inbox: a smart
 * filter or search query is always resolved against `mailbox IN ('inbox','sent')`
 * and ignores the selected mailbox, so keeping one alive while switching to
 * Sent/Spam/Trash/a folder would highlight a view whose emails never appear.
 */
export function planViewChange(mode: ViewMode, layout: InboxLayout): ViewChangePlan {
  return {
    resetInboxFilters: isEmailListView(mode),
    closeOpenEmail: layout === 'full-width',
  };
}
