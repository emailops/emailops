// The app-wide keyboard handler. Mounted once, at the app root.
//
// `matchKey` (src/lib/shortcuts.ts) turns a key press into a shortcut id,
// `planShortcut` (src/lib/shortcutPlan.ts) turns the id into an effect for the
// current UI state, and this hook executes the effect: store actions for list
// and selection work, a pane command for the open conversation (EmailView
// owns its reply/forward/snooze handlers), and the host's callbacks for the
// things only App can do (open a row, compose, search, change view).

import { useEffect, useRef } from 'react';
import type { ViewMode } from '@/components/Sidebar/Sidebar';
import * as api from '@/lib/api';
import { bulkAvailability } from '@/lib/bulkActions';
import { planShortcut, type ShortcutEffect, type ThreadShortcutAction } from '@/lib/shortcutPlan';
import { createShortcutMatcher } from '@/lib/shortcuts';
import { threadRefOf, useEmailStore } from '@/stores/emailStore';
import { anyOverlayOpen, useOverlayStore } from '@/stores/overlayStore';
import { selectedThreads, useSelectionStore } from '@/stores/selectionStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email, InboxLayout } from '@/types';

/** What the app root tells the handler about the screen, plus what it can do. */
export interface GlobalShortcutHost {
  /** The main area shows an email list. */
  listView: boolean;
  layout: InboxLayout;
  /** A conversation is shown in the reading pane. */
  openConversation: boolean;
  /** The list row of the open conversation, if it came from the list. */
  openEmailId: string | null;
  /** Open a row the way a click does (marks it read). */
  openEmail: (email: Email) => void;
  /** Leave the open conversation (full-width: back to the list). */
  closeConversation: () => void;
  compose: () => void;
  /** `/`: put the caret in the search field. */
  focusSearch: () => void;
  /** Cmd/Ctrl+K: the search overlay. */
  openSearchPalette: () => void;
  goTo: (view: ViewMode) => void;
}

const TEXT_INPUT_TYPES = new Set([
  'text',
  'search',
  'email',
  'url',
  'tel',
  'password',
  'number',
  'date',
  'datetime-local',
  'month',
  'time',
  'week',
]);

/** Focus is somewhere keys mean text: a text input, textarea, select or a
 *  contenteditable (the TipTap composer). */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target instanceof HTMLInputElement) return TEXT_INPUT_TYPES.has(target.type || 'text');
  if (target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return true;
  if (target.isContentEditable) return true;
  return target.closest('[contenteditable]:not([contenteditable="false"])') !== null;
}

/** Enter (or Space) on a focused control belongs to that control. */
function isActivatingControl(event: KeyboardEvent): boolean {
  if (event.key !== 'Enter' && event.key !== ' ') return false;
  const target = event.target;
  return target instanceof HTMLElement && target.closest('button, a[href], [role="button"], summary') !== null;
}

/** An overlay is on screen: it owns the keyboard (Escape, Enter, typing), so
 *  no conversation shortcut may run. Overlays say so explicitly through
 *  `useOverlay` (src/stores/overlayStore.ts); `aria-modal` still counts for any
 *  dialog that declares itself the standard way. */
export function isOverlayOpen(): boolean {
  return anyOverlayOpen(useOverlayStore.getState()) || document.querySelector('[aria-modal="true"]') !== null;
}

function runThreadAction(action: ThreadShortcutAction, rows: Email[], fromSelection: boolean) {
  const store = useEmailStore.getState();
  const threads = fromSelection
    ? selectedThreads({ ids: new Set(rows.map((e) => e.id)), anchor: null }, rows)
    : rows.map(threadRefOf);
  if (threads.length === 0) return;
  const can = bulkAvailability(rows);
  const clear = () => {
    if (fromSelection) useSelectionStore.getState().clear();
  };
  switch (action) {
    case 'archive':
      if (!can.canArchive) return;
      clear();
      void store.archiveThreads(threads);
      return;
    case 'delete':
      clear();
      void store.deleteThreads(threads);
      return;
    case 'star':
      void store.setThreadsStarred(threads, can.canStar);
      return;
    case 'markRead':
      void store.setThreadsRead(threads, true);
      return;
    case 'markUnread':
      void store.setThreadsRead(threads, false);
      return;
    case 'snooze':
      if (!can.canSnooze) return;
      // The picker lives in the bulk toolbar, which shows while rows are
      // selected: a cursor row joins the selection to get there.
      if (!fromSelection) useSelectionStore.getState().toggle(rows[0].id);
      useShortcutStore.getState().requestBulkSnooze();
      return;
  }
}

function execute(effect: ShortcutEffect, host: GlobalShortcutHost, list: Email[]) {
  const shortcuts = useShortcutStore.getState();
  const selection = useSelectionStore.getState();
  const byId = (id: string) => list.find((e) => e.id === id);
  switch (effect.type) {
    case 'none':
      return;
    case 'cursor':
      shortcuts.setCursor(effect.id);
      return;
    case 'open': {
      const email = byId(effect.id);
      if (!email) return;
      shortcuts.setCursor(effect.id);
      host.openEmail(email);
      return;
    }
    case 'closeConversation':
      host.closeConversation();
      return;
    case 'clearSelection':
      selection.clear();
      return;
    case 'toggleSelect':
      selection.toggle(effect.id);
      return;
    case 'selectAll':
      selection.selectAll(list.map((e) => e.id));
      return;
    case 'bulk':
      runThreadAction(
        effect.action,
        list.filter((e) => selection.ids.has(e.id)),
        true,
      );
      return;
    case 'row': {
      const email = byId(effect.id);
      if (email) runThreadAction(effect.action, [email], false);
      return;
    }
    case 'pane':
      shortcuts.requestPaneCommand(effect.command);
      return;
    case 'compose':
      host.compose();
      return;
    case 'search':
      host.focusSearch();
      return;
    case 'searchPalette':
      host.openSearchPalette();
      return;
    case 'help':
      shortcuts.setHelpOpen(true);
      return;
    case 'goTo':
      host.goTo(effect.view);
      return;
  }
}

export interface GlobalShortcutOptions {
  /** Injected clock for the two-key sequence timeout (tests). */
  now?: () => number;
}

export function useGlobalShortcuts(host: GlobalShortcutHost, options: GlobalShortcutOptions = {}) {
  const hostRef = useRef(host);
  hostRef.current = host;
  const nowRef = useRef(options.now ?? Date.now);
  nowRef.current = options.now ?? Date.now;

  useEffect(() => {
    const matcher = createShortcutMatcher(() => nowRef.current());
    const platform = api.currentPlatform();
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if (!useShortcutStore.getState().enabled) return;
      const editable = isEditableTarget(event.target);
      const id = matcher.match(event, { platform, editable, modalOpen: isOverlayOpen(), scope: 'global' });
      if (id === null || isActivatingControl(event)) return;
      const current = hostRef.current;
      const { listEmails, cursorId } = useShortcutStore.getState();
      const effect = planShortcut(id, {
        listView: current.listView,
        layout: current.layout,
        listIds: listEmails.map((e) => e.id),
        selectionCount: useSelectionStore.getState().ids.size,
        openConversation: current.openConversation,
        openEmailId: current.openEmailId,
        cursorId,
      });
      if (effect.type === 'none') return;
      event.preventDefault();
      execute(effect, current, listEmails);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, []);
}
