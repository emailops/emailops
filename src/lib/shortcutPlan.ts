// Pure planner: what a matched shortcut does in the current UI state. The
// global key handler (`useGlobalShortcuts`) executes the effect against the
// stores and the app's callbacks; every decision lives here so it is tested
// without React.

import type { ViewMode } from '@/components/Sidebar/Sidebar';
import type { InboxLayout } from '@/types';
import type { ShortcutId } from './shortcuts';

/** Actions on whole conversations, shared by the selection, the open
 *  conversation and the cursor row. */
export type ThreadShortcutAction = 'archive' | 'delete' | 'star' | 'markUnread' | 'markRead' | 'snooze';

/** What the reading pane is asked to do with the conversation it shows. */
export type PaneCommand = ThreadShortcutAction | 'reply' | 'replyAll' | 'forward';

export interface ShortcutPlanContext {
  /** The main area shows an email list (inbox, a mailbox, a folder). */
  listView: boolean;
  layout: InboxLayout;
  /** Ids of the rows the list shows, in display order. */
  listIds: readonly string[];
  /** Rows checked in the multi-selection. */
  selectionCount: number;
  /** A conversation is shown in the reading pane. */
  openConversation: boolean;
  /** The list row of the open conversation, when it came from the list. */
  openEmailId: string | null;
  /** The keyboard cursor row in the list. */
  cursorId: string | null;
}

export type ShortcutEffect =
  | { type: 'none' }
  /** Move the keyboard cursor without opening (full-width list). */
  | { type: 'cursor'; id: string }
  /** Open a row in the reading pane (and move the cursor to it). */
  | { type: 'open'; id: string }
  | { type: 'closeConversation' }
  | { type: 'clearSelection' }
  | { type: 'toggleSelect'; id: string }
  | { type: 'selectAll' }
  | { type: 'bulk'; action: ThreadShortcutAction }
  | { type: 'pane'; command: PaneCommand }
  | { type: 'row'; action: ThreadShortcutAction; id: string }
  | { type: 'compose' }
  | { type: 'search' }
  | { type: 'searchPalette' }
  | { type: 'help' }
  | { type: 'goTo'; view: ViewMode };

const NONE: ShortcutEffect = { type: 'none' };

const GO_TO: Partial<Record<ShortcutId, ViewMode>> = {
  'go.inbox': 'inbox',
  'go.starred': 'starred',
  'go.sent': 'sent',
  'go.drafts': 'drafts',
  'go.snoozed': 'snoozed',
  'go.archive': 'archive',
  'go.scheduled': 'scheduled',
  'go.contacts': 'contacts',
};

const THREAD_ACTIONS: Partial<Record<ShortcutId, ThreadShortcutAction>> = {
  'thread.archive': 'archive',
  'thread.delete': 'delete',
  'thread.star': 'star',
  'thread.markUnread': 'markUnread',
  'thread.markRead': 'markRead',
  'thread.snooze': 'snooze',
};

const COMPOSE_COMMANDS: Partial<Record<ShortcutId, PaneCommand>> = {
  'compose.reply': 'reply',
  'compose.replyAll': 'replyAll',
  'compose.forward': 'forward',
};

/** The full-width list is on screen (no conversation replacing it). */
const browsingFullWidthList = (c: ShortcutPlanContext) => c.layout === 'full-width' && !c.openConversation;

/** The row `x` and Enter talk about: the cursor while browsing the full-width
 *  list, otherwise the open row (falling back to the cursor). */
function focusedRow(c: ShortcutPlanContext): string | null {
  const id = browsingFullWidthList(c) ? c.cursorId : (c.openEmailId ?? c.cursorId);
  return id !== null && c.listIds.includes(id) ? id : null;
}

function step(c: ShortcutPlanContext, delta: 1 | -1): ShortcutEffect {
  if (!c.listView || c.listIds.length === 0) return NONE;
  const browsing = browsingFullWidthList(c);
  const from = browsing ? c.cursorId : (c.openEmailId ?? (c.openConversation ? null : c.cursorId));
  const index = from === null ? -1 : c.listIds.indexOf(from);
  let target: string;
  if (index === -1) {
    // A conversation opened from outside the list has no neighbour in it.
    if (c.openConversation && !browsing) return NONE;
    target = c.listIds[0];
  } else {
    const next = index + delta;
    if (next < 0 || next >= c.listIds.length) return NONE;
    target = c.listIds[next];
  }
  return browsing ? { type: 'cursor', id: target } : { type: 'open', id: target };
}

export function planShortcut(id: ShortcutId, c: ShortcutPlanContext): ShortcutEffect {
  const goTo = GO_TO[id];
  if (goTo) return { type: 'goTo', view: goTo };

  const threadAction = THREAD_ACTIONS[id];
  if (threadAction) {
    if (c.listView && c.selectionCount > 0) return { type: 'bulk', action: threadAction };
    if (c.openConversation) return { type: 'pane', command: threadAction };
    if (c.listView && browsingFullWidthList(c)) {
      const row = focusedRow(c);
      return row ? { type: 'row', action: threadAction, id: row } : NONE;
    }
    return NONE;
  }

  const composeCommand = COMPOSE_COMMANDS[id];
  if (composeCommand) return c.openConversation ? { type: 'pane', command: composeCommand } : NONE;

  switch (id) {
    case 'nav.next':
      return step(c, 1);
    case 'nav.prev':
      return step(c, -1);
    case 'nav.open': {
      if (!c.listView) return NONE;
      const row = browsingFullWidthList(c) ? focusedRow(c) : c.cursorId;
      if (row === null || !c.listIds.includes(row) || row === c.openEmailId) return NONE;
      return { type: 'open', id: row };
    }
    case 'nav.back':
      return c.layout === 'full-width' && c.openConversation ? { type: 'closeConversation' } : NONE;
    case 'nav.escape':
      if (c.selectionCount > 0) return { type: 'clearSelection' };
      return c.layout === 'full-width' && c.openConversation ? { type: 'closeConversation' } : NONE;
    case 'select.toggle': {
      if (!c.listView) return NONE;
      const row = focusedRow(c);
      return row ? { type: 'toggleSelect', id: row } : NONE;
    }
    case 'select.all':
      return c.listView && c.listIds.length > 0 ? { type: 'selectAll' } : NONE;
    case 'select.none':
      return c.selectionCount > 0 ? { type: 'clearSelection' } : NONE;
    case 'compose.new':
      return { type: 'compose' };
    case 'app.search':
      return { type: 'search' };
    case 'app.searchPalette':
      return { type: 'searchPalette' };
    case 'app.help':
      return { type: 'help' };
    default:
      return NONE;
  }
}
