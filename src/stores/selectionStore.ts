import { create } from 'zustand';
import type { ThreadRef } from '@/lib/api';
import type { Email } from '@/types';
import { threadKey } from './emailStore';

/**
 * Multi-select in the email list. Rows are selected by email id (what the
 * list renders); the bulk actions act on the conversations those rows stand
 * for (`selectedThreads`). State transitions are pure reducers so the list,
 * the bulk toolbar and keyboard shortcuts share them.
 */
export interface SelectionState {
  ids: ReadonlySet<string>;
  /** The row the last plain or Cmd/Ctrl click landed on — where a
   *  shift-click range starts. */
  anchor: string | null;
}

export const EMPTY_SELECTION: SelectionState = { ids: new Set(), anchor: null };

/** Pure: add or remove one row; it becomes the range anchor. */
export function toggleSelection(state: SelectionState, id: string): SelectionState {
  const ids = new Set(state.ids);
  if (ids.has(id)) ids.delete(id);
  else ids.add(id);
  return { ids, anchor: id };
}

/**
 * Pure: shift-click — add every row between the anchor and `id` (inclusive,
 * in list order) to the selection. With no anchor in the list it is a toggle.
 */
export function selectRange(state: SelectionState, order: readonly string[], id: string): SelectionState {
  const from = state.anchor === null ? -1 : order.indexOf(state.anchor);
  const to = order.indexOf(id);
  if (from === -1 || to === -1) return toggleSelection(state, id);
  const ids = new Set(state.ids);
  const [lo, hi] = from <= to ? [from, to] : [to, from];
  for (let i = lo; i <= hi; i++) ids.add(order[i]);
  return { ids, anchor: state.anchor };
}

/** Pure: select every loaded row. */
export function selectAll(state: SelectionState, order: readonly string[]): SelectionState {
  return { ids: new Set(order), anchor: state.anchor };
}

export function clearSelection(): SelectionState {
  return EMPTY_SELECTION;
}

/**
 * Pure: forget rows that are no longer in the list (archived, deleted, a
 * refetch dropped them). Returns `state` itself when nothing changed, so a
 * subscriber does not re-render.
 */
export function pruneSelection(state: SelectionState, present: readonly string[]): SelectionState {
  const here = new Set(present);
  const anchor = state.anchor !== null && here.has(state.anchor) ? state.anchor : null;
  let changed = anchor !== state.anchor;
  const ids = new Set<string>();
  for (const id of state.ids) {
    if (here.has(id)) ids.add(id);
    else changed = true;
  }
  return changed ? { ids, anchor } : state;
}

/** The conversations of the selected rows, once each, in list order. */
export function selectedThreads(state: SelectionState, emails: readonly Email[]): ThreadRef[] {
  const seen = new Set<string>();
  const refs: ThreadRef[] = [];
  for (const e of emails) {
    if (!state.ids.has(e.id)) continue;
    const key = threadKey(e.accountId, e.threadId);
    if (seen.has(key)) continue;
    seen.add(key);
    refs.push({ accountId: e.accountId, threadId: e.threadId });
  }
  return refs;
}

interface SelectionStore extends SelectionState {
  toggle: (id: string) => void;
  selectRange: (order: readonly string[], id: string) => void;
  selectAll: (order: readonly string[]) => void;
  clear: () => void;
  prune: (present: readonly string[]) => void;
}

const stateOf = (s: SelectionStore): SelectionState => ({ ids: s.ids, anchor: s.anchor });

export const useSelectionStore = create<SelectionStore>((set) => ({
  ...EMPTY_SELECTION,
  toggle: (id) => set((s) => toggleSelection(stateOf(s), id)),
  selectRange: (order, id) => set((s) => selectRange(stateOf(s), order, id)),
  selectAll: (order) => set((s) => selectAll(stateOf(s), order)),
  clear: () => set((s) => (s.ids.size === 0 && s.anchor === null ? s : clearSelection())),
  prune: (present) =>
    set((s) => {
      const next = pruneSelection(stateOf(s), present);
      return next.ids === s.ids && next.anchor === s.anchor ? s : next;
    }),
}));
