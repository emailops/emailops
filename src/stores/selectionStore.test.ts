// Multi-select in the email list: pure reducers (toggle, range, all, clear,
// prune) and the selector that turns a selection into thread refs.

import { beforeEach, describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import {
  clearSelection,
  EMPTY_SELECTION,
  pruneSelection,
  selectAll,
  selectedThreads,
  selectRange,
  toggleSelection,
  useSelectionStore,
} from './selectionStore';

const ORDER = ['a', 'b', 'c', 'd', 'e'];

describe('toggleSelection', () => {
  it('adds and removes an id and moves the anchor to it', () => {
    const one = toggleSelection(EMPTY_SELECTION, 'b');
    expect([...one.ids]).toEqual(['b']);
    expect(one.anchor).toBe('b');

    const none = toggleSelection(one, 'b');
    expect(none.ids.size).toBe(0);
    expect(none.anchor).toBe('b');
  });
});

describe('selectRange', () => {
  it('selects from the anchor to the clicked row, in either direction', () => {
    const anchored = toggleSelection(EMPTY_SELECTION, 'b');
    expect([...selectRange(anchored, ORDER, 'd').ids].sort()).toEqual(['b', 'c', 'd']);

    const back = toggleSelection(EMPTY_SELECTION, 'd');
    expect([...selectRange(back, ORDER, 'a').ids].sort()).toEqual(['a', 'b', 'c', 'd']);
  });

  it('keeps what was already selected outside the range', () => {
    const state = toggleSelection(toggleSelection(EMPTY_SELECTION, 'e'), 'a');
    expect([...selectRange(state, ORDER, 'b').ids].sort()).toEqual(['a', 'b', 'e']);
  });

  it('without an anchor (or a stale one) behaves as a toggle', () => {
    expect([...selectRange(EMPTY_SELECTION, ORDER, 'c').ids]).toEqual(['c']);
    const stale = { ids: new Set<string>(), anchor: 'gone' };
    expect([...selectRange(stale, ORDER, 'c').ids]).toEqual(['c']);
  });
});

describe('selectAll / clearSelection', () => {
  it('selects every loaded row, and clears', () => {
    const all = selectAll(EMPTY_SELECTION, ORDER);
    expect(all.ids.size).toBe(5);
    expect(clearSelection().ids.size).toBe(0);
  });
});

describe('pruneSelection', () => {
  it('drops ids that left the list and keeps the same object when nothing changed', () => {
    const state = selectAll(EMPTY_SELECTION, ['a', 'b', 'c']);
    const pruned = pruneSelection(state, ['a', 'c', 'z']);
    expect([...pruned.ids].sort()).toEqual(['a', 'c']);
    expect(pruneSelection(pruned, ['a', 'c'])).toBe(pruned);
  });

  it('forgets an anchor that left the list', () => {
    const state = toggleSelection(EMPTY_SELECTION, 'b');
    expect(pruneSelection(state, ['a']).anchor).toBeNull();
  });
});

describe('selectedThreads', () => {
  const row = (id: string, threadId: string, accountId = 'acc') => ({ id, threadId, accountId }) as Email;

  it('maps selected rows to their conversations, once each, in list order', () => {
    const emails = [row('a', 't1'), row('b', 't2'), row('c', 't1'), row('d', 't3', 'other')];
    const state = selectAll(EMPTY_SELECTION, ['d', 'c', 'a']);
    expect(selectedThreads(state, emails)).toEqual([
      { accountId: 'acc', threadId: 't1' },
      { accountId: 'other', threadId: 't3' },
    ]);
  });
});

describe('useSelectionStore', () => {
  beforeEach(() => useSelectionStore.getState().clear());

  it('exposes the reducers as actions (also for keyboard shortcuts)', () => {
    const s = useSelectionStore.getState();
    s.toggle('a');
    s.selectRange(ORDER, 'c');
    expect([...useSelectionStore.getState().ids].sort()).toEqual(['a', 'b', 'c']);
    s.prune(['a']);
    expect([...useSelectionStore.getState().ids]).toEqual(['a']);
    s.selectAll(ORDER);
    expect(useSelectionStore.getState().ids.size).toBe(5);
    s.clear();
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });
});
