// Auto-advance: which conversation opens after the open one leaves the list
// (archive, delete, snooze, block, mark as spam). Gmail's "Auto-advance",
// Outlook's "after moving or deleting an item". Pure; the executor lives in
// `src/stores/autoAdvanceStore.ts`.

/** `next` / `previous`: open that neighbour in list order. `list`: close the
 *  conversation and go back to the list. */
export type AfterLeaveMode = 'next' | 'previous' | 'list';

export const AFTER_LEAVE_MODES: readonly AfterLeaveMode[] = ['next', 'previous', 'list'];

/** SQLite preference holding the mode. */
export const AFTER_LEAVE_PREF = 'ui.after_thread_leave';

/** Default: open the next conversation (docs/DECISIONS.md, "Auto-advance"). */
export const DEFAULT_AFTER_LEAVE_MODE: AfterLeaveMode = 'next';

export function parseAfterLeaveMode(raw: string | null): AfterLeaveMode {
  return AFTER_LEAVE_MODES.find((m) => m === raw) ?? DEFAULT_AFTER_LEAVE_MODE;
}

/**
 * The row to open once `leavingId` has left: its neighbour in `listIds` (the
 * list as the user saw it when acting), skipping rows in `gone` (rows that
 * left with it). `next` falls back to the previous row at the end of the list
 * and `previous` to the next one at the top. `null`: go back to the list —
 * by choice, because the list is empty, or because the conversation was not
 * opened from this list.
 */
export function planAdvance(
  listIds: readonly string[],
  leavingId: string,
  gone: ReadonlySet<string>,
  mode: AfterLeaveMode,
): string | null {
  if (mode === 'list') return null;
  const index = listIds.indexOf(leavingId);
  if (index === -1) return null;
  const stays = (id: string) => id !== leavingId && !gone.has(id);
  const after = listIds.slice(index + 1).find(stays) ?? null;
  const before = [...listIds.slice(0, index)].reverse().find(stays) ?? null;
  return mode === 'next' ? (after ?? before) : (before ?? after);
}
