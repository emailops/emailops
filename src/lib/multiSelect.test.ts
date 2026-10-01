import { describe, expect, it } from 'vitest';
import {
  applyClick,
  clickModifiers,
  EMPTY_SELECTION,
  isAllSelected,
  isMultiSelecting,
  type MultiSelection,
  pruneSelection,
  selectAll,
  toggleSelectAll,
} from './multiSelect';

const ORDER = ['a', 'b', 'c', 'd', 'e'];
const PLAIN = { toggle: false, range: false };
const CTRL = { toggle: true, range: false };
const SHIFT = { toggle: false, range: true };
const CTRL_SHIFT = { toggle: true, range: true };

function sel(
  ids: string[],
  anchor: string | null = ids[ids.length - 1] ?? null,
  mode = ids.length > 1,
): MultiSelection {
  return { ids, anchor, mode };
}

describe('clickModifiers', () => {
  it('uses ⌘ on macOS and Ctrl elsewhere', () => {
    expect(clickModifiers({ ctrlKey: true, metaKey: false, shiftKey: false }, false).toggle).toBe(true);
    expect(clickModifiers({ ctrlKey: true, metaKey: false, shiftKey: false }, true).toggle).toBe(false);
    expect(clickModifiers({ ctrlKey: false, metaKey: true, shiftKey: false }, true).toggle).toBe(true);
    expect(clickModifiers({ ctrlKey: false, metaKey: false, shiftKey: true }, false).range).toBe(true);
  });
});

describe('applyClick outside selection mode', () => {
  it('a plain click opens the email', () => {
    expect(applyClick(EMPTY_SELECTION, 'c', PLAIN, ORDER, 'a')).toEqual({
      kind: 'open',
      selection: { ids: [], anchor: 'c', mode: false },
    });
  });

  it('Ctrl+click after opening an email selects both and enters the mode', () => {
    expect(applyClick(EMPTY_SELECTION, 'c', CTRL, ORDER, 'a')).toEqual({
      kind: 'select',
      selection: { ids: ['a', 'c'], anchor: 'c', mode: true },
    });
  });

  it('Shift+click selects the range from the anchor, in either direction', () => {
    expect(applyClick(EMPTY_SELECTION, 'd', SHIFT, ORDER, 'b').selection).toEqual({
      ids: ['b', 'c', 'd'],
      anchor: 'b',
      mode: true,
    });
    expect(applyClick(EMPTY_SELECTION, 'a', SHIFT, ORDER, 'c').selection.ids).toEqual(['a', 'b', 'c']);
  });

  it('Shift+click with nothing open selects just that email, without the mode', () => {
    expect(applyClick(EMPTY_SELECTION, 'c', SHIFT, ORDER, null).selection).toEqual({
      ids: ['c'],
      anchor: 'c',
      mode: false,
    });
  });
});

describe('applyClick in selection mode', () => {
  it('Ctrl+click toggles an email in and out', () => {
    const added = applyClick(sel(['a', 'c']), 'e', CTRL, ORDER, 'a').selection;
    expect(added.ids).toEqual(['a', 'c', 'e']);
    const removed = applyClick(added, 'c', CTRL, ORDER, 'a').selection;
    expect(removed.ids).toEqual(['a', 'e']);
  });

  it('a plain click toggles a row instead of opening it', () => {
    const r = applyClick(sel(['a', 'c']), 'd', PLAIN, ORDER, 'a');
    expect(r.kind).toBe('select');
    expect(r.selection.ids).toEqual(['a', 'c', 'd']);
    expect(applyClick(r.selection, 'a', PLAIN, ORDER, 'a').selection.ids).toEqual(['c', 'd']);
  });

  it('stays in the mode when the selection is emptied, and a click adds back', () => {
    const empty = applyClick(sel(['a', 'c']), 'a', PLAIN, ORDER, null).selection;
    const none = applyClick(empty, 'c', PLAIN, ORDER, null).selection;
    expect(none).toEqual({ ids: [], anchor: 'c', mode: true });
    // The open email is not pulled back in: in the mode, the selection is explicit.
    expect(applyClick(none, 'e', PLAIN, ORDER, 'a').selection.ids).toEqual(['e']);
  });

  it('a second Shift+click re-ranges from the same anchor', () => {
    const first = applyClick(EMPTY_SELECTION, 'e', SHIFT, ORDER, 'b').selection;
    expect(first.ids).toEqual(['b', 'c', 'd', 'e']);
    expect(applyClick(first, 'c', SHIFT, ORDER, 'b').selection.ids).toEqual(['b', 'c']);
  });

  it('Ctrl+Shift+click adds a range to the existing selection', () => {
    const r = applyClick(sel(['a'], 'c', true), 'e', CTRL_SHIFT, ORDER, null).selection;
    expect(r.ids).toEqual(['a', 'c', 'd', 'e']);
  });

  it('an anchor that is no longer visible falls back to the clicked email', () => {
    expect(applyClick(sel(['z'], 'z', true), 'b', SHIFT, ORDER, null).selection.ids).toEqual(['b']);
  });
});

describe('select all', () => {
  it('selectAll takes every visible email and enters the mode', () => {
    expect(selectAll(ORDER)).toEqual({ ids: ORDER, anchor: 'a', mode: true });
  });

  it('the ✓ button toggles: all, then none — staying in the mode', () => {
    const all = toggleSelectAll(sel(['a', 'b']), ORDER);
    expect(all.ids).toEqual(ORDER);
    expect(isAllSelected(all, ORDER)).toBe(true);
    const none = toggleSelectAll(all, ORDER);
    expect(none).toEqual({ ids: [], anchor: null, mode: true });
    expect(isMultiSelecting(none)).toBe(true);
    expect(toggleSelectAll(none, ORDER).ids).toEqual(ORDER);
  });

  it('isAllSelected is false for an empty list or a partial selection', () => {
    expect(isAllSelected(sel(['a', 'b']), ORDER)).toBe(false);
    expect(isAllSelected(sel([]), [])).toBe(false);
  });
});

describe('pruneSelection / isMultiSelecting', () => {
  it('drops ids that left the list, keeps the mode, and keeps the same object when nothing changed', () => {
    const s = sel(['a', 'x', 'c'], 'x', true);
    expect(pruneSelection(s, ORDER)).toEqual({ ids: ['a', 'c'], anchor: null, mode: true });
    const unchanged = sel(['a', 'b'], 'b');
    expect(pruneSelection(unchanged, ORDER)).toBe(unchanged);
  });

  it('the mode starts at two emails and is not left by itself', () => {
    expect(isMultiSelecting(applyClick(EMPTY_SELECTION, 'b', CTRL, ORDER, null).selection)).toBe(false);
    const two = applyClick(EMPTY_SELECTION, 'b', CTRL, ORDER, 'a').selection;
    expect(isMultiSelecting(two)).toBe(true);
    const one = applyClick(two, 'b', CTRL, ORDER, 'a').selection;
    expect(one.ids).toEqual(['a']);
    expect(isMultiSelecting(one)).toBe(true);
  });
});
