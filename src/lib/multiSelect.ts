/**
 * Multi-selection of email rows in the list: pure state transitions, so the
 * click rules are unit-tested without React (see `multiSelect.test.ts`).
 *
 * Conventions follow Thunderbird / Outlook / file managers:
 *   - plain click          → open that email, clear the multi-selection
 *   - Ctrl/⌘ + click       → add or remove that email (anchor moves to it)
 *   - Shift + click        → select the range from the anchor to it
 *   - Ctrl/⌘ + Shift + click → add that range to the current selection
 *
 * The selection is an ordered set of email ids; `anchor` is where the next
 * Shift range starts. Ids are resolved against the *visible* list order, so a
 * range never includes hidden or filtered-out emails.
 *
 * `mode` is the selection *mode* (the action bar is shown). It starts when
 * two emails are selected and lasts until the user leaves it (✕ or Escape),
 * even if the selection is emptied in between — so "Select all" can be undone
 * without closing the bar. While in the mode, a plain click toggles a row
 * instead of opening it, as on a phone or in Gmail's checkbox mode.
 */

export interface MultiSelection {
  /** Selected email ids, in the order they were added. */
  ids: string[];
  /** Start of the next Shift range (the last email clicked with Ctrl or plain). */
  anchor: string | null;
  /** Selection mode is on: the action bar stays until the user leaves it. */
  mode: boolean;
}

export const EMPTY_SELECTION: MultiSelection = { ids: [], anchor: null, mode: false };

/** Selection state with the mode turned on once two or more emails are selected. */
function withMode(ids: string[], anchor: string | null, wasMode: boolean): MultiSelection {
  return { ids, anchor, mode: wasMode || ids.length > 1 };
}

export interface ClickModifiers {
  /** Ctrl on Linux/Windows, ⌘ on macOS. */
  toggle: boolean;
  range: boolean;
}

/** What a click on a row should do. */
export type ClickOutcome =
  /** Open this email normally (and the selection is now `selection`). */
  | { kind: 'open'; selection: MultiSelection }
  /** Only the selection changed; do not open anything. */
  | { kind: 'select'; selection: MultiSelection };

/** Read Ctrl/⌘ and Shift from a mouse event (⌘ on macOS, Ctrl elsewhere). */
export function clickModifiers(
  e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean },
  isMac: boolean,
): ClickModifiers {
  return { toggle: isMac ? e.metaKey : e.ctrlKey, range: e.shiftKey };
}

function rangeIds(order: string[], from: string, to: string): string[] {
  const a = order.indexOf(from);
  const b = order.indexOf(to);
  if (a === -1 || b === -1) return [to];
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  return order.slice(lo, hi + 1);
}

function union(first: string[], second: string[]): string[] {
  const seen = new Set(first);
  return [...first, ...second.filter((id) => !seen.has(id))];
}

/**
 * Apply a click on `clickedId` to the selection. `order` is the visible list
 * order; `openId` is the email currently open in the reading pane (it counts
 * as the anchor when nothing else is selected, so Ctrl/Shift-clicking right
 * after opening an email includes it, as in other mail clients).
 */
export function applyClick(
  state: MultiSelection,
  clickedId: string,
  mods: ClickModifiers,
  order: string[],
  openId: string | null,
): ClickOutcome {
  // Outside the mode, the open email is the implicit starting point.
  const base =
    state.ids.length > 0 || state.mode ? state : { ids: openId ? [openId] : [], anchor: openId, mode: false };

  if (mods.range) {
    const anchor = base.anchor && order.includes(base.anchor) ? base.anchor : clickedId;
    const range = rangeIds(order, anchor, clickedId);
    const ids = mods.toggle ? union(base.ids, range) : range;
    // Shift keeps the anchor, so the next Shift-click re-ranges from the same start.
    return { kind: 'select', selection: withMode(ids, anchor, base.mode) };
  }

  // Ctrl/⌘+click toggles; in selection mode, so does a plain click.
  if (mods.toggle || base.mode) {
    const ids = base.ids.includes(clickedId) ? base.ids.filter((id) => id !== clickedId) : [...base.ids, clickedId];
    return { kind: 'select', selection: withMode(ids, clickedId, base.mode) };
  }

  return { kind: 'open', selection: { ids: [], anchor: clickedId, mode: false } };
}

/** Keep only ids still present in the list (after a delete, move or filter change). */
export function pruneSelection(state: MultiSelection, order: string[]): MultiSelection {
  const present = new Set(order);
  const ids = state.ids.filter((id) => present.has(id));
  const anchor = state.anchor && present.has(state.anchor) ? state.anchor : null;
  if (ids.length === state.ids.length && anchor === state.anchor) return state;
  return { ids, anchor, mode: state.mode };
}

/** Select every visible email. */
export function selectAll(order: string[]): MultiSelection {
  return { ids: [...order], anchor: order[0] ?? null, mode: true };
}

/** Whether every visible email is selected (and there is at least one). */
export function isAllSelected(state: MultiSelection, order: string[]): boolean {
  if (order.length === 0) return false;
  const selected = new Set(state.ids);
  return order.every((id) => selected.has(id));
}

/**
 * The ✓ button: select every visible email, or — when they already all are —
 * deselect them. Either way the selection mode (and its bar) stays on.
 */
export function toggleSelectAll(state: MultiSelection, order: string[]): MultiSelection {
  if (isAllSelected(state, order)) return { ids: [], anchor: null, mode: true };
  return selectAll(order);
}

/** Whether the selection mode is on (the action bar is shown). */
export function isMultiSelecting(state: MultiSelection): boolean {
  return state.mode;
}
