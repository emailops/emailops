// The undo window of archive and delete (and the Undo of snooze): how long the
// Undo notice stays up and the provider call waits. Pure; the preference
// lives in `src/stores/undoWindowStore.ts`, the queue in `pendingActions.ts`.
// Separate from Undo send (`src/lib/outbox.ts`).

/** Preference key (SQLite user prefs) holding the window in seconds. */
export const UNDO_WINDOW_PREF = 'ui.archive_delete_undo_secs';

/** The windows Settings offers; 0 = off (archive and delete act at once,
 *  without Undo). Undo send's 10/20/30 plus the 6 s default. */
export const UNDO_WINDOW_OPTIONS = [0, 6, 10, 20, 30] as const;

export const DEFAULT_UNDO_WINDOW_SECS = 6;

/** The stored preference as a window in seconds; unset or unknown values are
 *  the default. */
export function parseUndoWindow(raw: string | null | undefined): number {
  if (raw === null || raw === undefined || raw.trim() === '') return DEFAULT_UNDO_WINDOW_SECS;
  const secs = Number(raw);
  return (UNDO_WINDOW_OPTIONS as readonly number[]).includes(secs) ? secs : DEFAULT_UNDO_WINDOW_SECS;
}
