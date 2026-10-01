import { create } from 'zustand';
import * as api from '@/lib/api';
import type { PaneCommand } from '@/lib/shortcutPlan';
import type { Email } from '@/types';

/** SQLite preference: keyboard shortcuts on (`true`, the default) or off. */
export const SHORTCUTS_ENABLED_PREF = 'ui.keyboard_shortcuts_enabled';

/** Pure: shortcuts stay on unless the preference says exactly `false`. */
export function parseShortcutsEnabled(raw: string | null): boolean {
  return raw !== 'false';
}

/** A one-shot request: a new `nonce` every time, so the same command twice
 *  in a row is still two events. */
export interface PaneCommandRequest {
  command: PaneCommand;
  nonce: number;
}

interface ShortcutStore {
  enabled: boolean;
  loadEnabled: () => Promise<void>;
  /** Persist the setting; throws (and keeps the old value) when it cannot. */
  setEnabled: (enabled: boolean) => Promise<void>;

  helpOpen: boolean;
  setHelpOpen: (open: boolean) => void;

  /** Last command for the reading pane's open conversation (`EmailView`). */
  paneCommand: PaneCommandRequest | null;
  requestPaneCommand: (command: PaneCommand) => void;
  /** Ask the bulk toolbar to open its snooze picker. A flag, not a counter:
   *  the toolbar may mount after the request (the `b` on a cursor row selects
   *  it first), so it consumes the request whenever it sees it. */
  bulkSnoozeRequested: boolean;
  requestBulkSnooze: () => void;
  consumeBulkSnooze: () => void;

  /** The rows the email list shows, in order — published by `Inbox` so the
   *  root key handler can walk them. Empty when no list is mounted. */
  listEmails: Email[];
  setListEmails: (emails: Email[]) => void;
  /** The keyboard cursor row of the list. */
  cursorId: string | null;
  setCursor: (id: string | null) => void;
}

let nonce = 0;

export const useShortcutStore = create<ShortcutStore>((set) => ({
  enabled: true,
  loadEnabled: async () => {
    try {
      set({ enabled: parseShortcutsEnabled(await api.getPref(SHORTCUTS_ENABLED_PREF)) });
    } catch (err) {
      // Unreadable preference: keep the default (on) rather than silently
      // disabling the keyboard.
      console.error('Could not read the keyboard-shortcuts setting', err);
    }
  },
  setEnabled: async (enabled) => {
    await api.setPref(SHORTCUTS_ENABLED_PREF, String(enabled));
    set({ enabled });
  },

  helpOpen: false,
  setHelpOpen: (helpOpen) => set({ helpOpen }),

  paneCommand: null,
  requestPaneCommand: (command) => {
    nonce += 1;
    set({ paneCommand: { command, nonce } });
  },
  bulkSnoozeRequested: false,
  requestBulkSnooze: () => set({ bulkSnoozeRequested: true }),
  consumeBulkSnooze: () => set({ bulkSnoozeRequested: false }),

  listEmails: [],
  setListEmails: (listEmails) => set({ listEmails }),
  cursorId: null,
  setCursor: (cursorId) => set({ cursorId }),
}));
