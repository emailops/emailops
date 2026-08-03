import { describe, expect, it } from 'vitest';
import { INITIAL_SCROLL_RESTORE, planScrollRestore } from './scrollRestore';

// Regression cover for the inbox "blank band at the top after Back" bug.
//
// In full-width layout App.tsx keeps the inbox mounted and hides it with
// Tailwind `hidden` (display: none) while an email is open, with the stated
// intent of preserving scroll position. display:none does the opposite: the
// browser resets scrollTop to 0 and fires NO scroll event. And
// @tanstack/virtual-core only ever updates its scrollOffset from scroll events
// (observeOffset registers a listener and never reads scrollTop on attach), so
// on Back the virtualizer still believed the old offset while the container sat
// at 0 — it rendered the window for a scroll position the container wasn't at,
// leaving a blank band above the rows until the first real scroll resynced it.
//
// This planner owns the decision: never trust a scrollTop read while hidden,
// and put the saved position back when the container becomes visible again
// (which fires a scroll event, resyncing the virtualizer for free).

describe('planScrollRestore', () => {
  it('remembers the position while the list stays visible', () => {
    const result = planScrollRestore(INITIAL_SCROLL_RESTORE, { hidden: false, scrollTop: 640 });

    expect(result.state).toEqual({ saved: 640, hidden: false });
    expect(result.restoreTo).toBeNull();
  });

  // The critical case: display:none has already zeroed scrollTop by the time we
  // observe it. Saving that 0 would discard the position we need to restore.
  it('does not overwrite the saved position with the zero read while hidden', () => {
    const visible = planScrollRestore(INITIAL_SCROLL_RESTORE, { hidden: false, scrollTop: 640 }).state;

    const result = planScrollRestore(visible, { hidden: true, scrollTop: 0 });

    expect(result.state).toEqual({ saved: 640, hidden: true });
    expect(result.restoreTo).toBeNull();
  });

  it('restores the saved position when the list becomes visible again', () => {
    const visible = planScrollRestore(INITIAL_SCROLL_RESTORE, { hidden: false, scrollTop: 640 }).state;
    const hidden = planScrollRestore(visible, { hidden: true, scrollTop: 0 }).state;

    const result = planScrollRestore(hidden, { hidden: false, scrollTop: 0 });

    expect(result.restoreTo).toBe(640);
    expect(result.state).toEqual({ saved: 640, hidden: false });
  });

  it('does not write when the position already matches', () => {
    const hidden = { saved: 640, hidden: true };

    const result = planScrollRestore(hidden, { hidden: false, scrollTop: 640 });

    expect(result.restoreTo, 'a redundant scrollTop write would fire a pointless scroll event').toBeNull();
  });

  it('has nothing to restore when the list was never scrolled', () => {
    const hidden = { saved: 0, hidden: true };

    const result = planScrollRestore(hidden, { hidden: false, scrollTop: 0 });

    expect(result.restoreTo).toBeNull();
  });

  it('stays put while hidden', () => {
    const hidden = { saved: 640, hidden: true };

    const result = planScrollRestore(hidden, { hidden: true, scrollTop: 0 });

    expect(result).toEqual({ state: hidden, restoreTo: null });
  });

  // A genuine scroll-to-top while visible must be recorded, or Back would
  // wrongly restore a stale offset.
  it('records a scroll back to the top while visible', () => {
    const scrolled = { saved: 640, hidden: false };

    const result = planScrollRestore(scrolled, { hidden: false, scrollTop: 0 });

    expect(result.state).toEqual({ saved: 0, hidden: false });
    expect(result.restoreTo).toBeNull();
  });

  it('starts with nothing saved and visible', () => {
    expect(INITIAL_SCROLL_RESTORE).toEqual({ saved: 0, hidden: false });
  });
});
