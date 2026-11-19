// The archive/delete undo window setting (Settings → Appearance), read by the
// thread actions in `emailStore`. The rules are in `src/lib/undoWindow.ts`.

import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { DEFAULT_UNDO_WINDOW_SECS, parseUndoWindow, UNDO_WINDOW_PREF } from '@/lib/undoWindow';
import { useLogStore } from './logStore';

interface UndoWindowStore {
  /** The window in seconds; 0 = off. The default until `load` resolves. */
  secs: number;
  load: () => Promise<void>;
  /** Persist the setting; throws (and keeps the old value) when it cannot. */
  setSecs: (secs: number) => Promise<void>;
}

export const useUndoWindowStore = create<UndoWindowStore>((set) => ({
  secs: DEFAULT_UNDO_WINDOW_SECS,
  load: async () => {
    try {
      set({ secs: parseUndoWindow(await api.getPref(UNDO_WINDOW_PREF)) });
    } catch (err) {
      useLogStore.getState().addLog('error', 'system', `Could not read the undo window setting: ${errorText(err)}`);
    }
  },
  setSecs: async (secs) => {
    await api.setPref(UNDO_WINDOW_PREF, String(secs));
    set({ secs });
  },
}));

/** The window the next archive, delete or snooze gets, in ms (0 = off). */
export function undoWindowMs(): number {
  return useUndoWindowStore.getState().secs * 1000;
}
