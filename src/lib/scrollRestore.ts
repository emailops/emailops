// Scroll-position survival across a `display: none` hide.
//
// Hiding an element with `display: none` (Tailwind `hidden`) makes the browser
// reset its scrollTop to 0 — and it fires NO scroll event while doing so. Any
// component that hides a scroll container to "keep it mounted so scroll position
// is preserved" therefore loses exactly the thing it was trying to keep.
//
// It also silently desynchronises a windowing library: @tanstack/virtual-core
// updates its `scrollOffset` only from scroll events (`observeOffset` registers
// a listener and never reads scrollTop on attach; `getScrollOffset` keeps any
// existing non-null value), so after the hide it still believes the old offset
// while the container sits at 0 — rendering the row window for a position the
// container isn't at. That is the inbox "blank band above the rows after Back"
// bug.
//
// Restoring the saved scrollTop when the container becomes visible fixes both:
// the user lands where they were, and the write fires a scroll event that
// resyncs the virtualizer for free.
//
// Kept as a pure planner because jsdom has no layout — `clientHeight` is always
// 0 and `offsetParent` always null there, so the DOM half cannot be asserted
// directly, but this decision table can.

export interface ScrollRestoreState {
  /** Last scrollTop seen while the container was genuinely visible. */
  saved: number;
  /** Whether the container was hidden at the previous observation. */
  hidden: boolean;
}

export interface ScrollRestorePlan {
  state: ScrollRestoreState;
  /** scrollTop to write back, or null when no write is needed. */
  restoreTo: number | null;
}

export const INITIAL_SCROLL_RESTORE: ScrollRestoreState = { saved: 0, hidden: false };

/**
 * Fold one observation of the container into the restore state.
 *
 * `hidden` must be derived from layout (e.g. `clientHeight === 0` or
 * `offsetParent === null`), not from React state, so it reflects what the
 * browser actually did to scrollTop.
 */
export function planScrollRestore(
  prev: ScrollRestoreState,
  next: { hidden: boolean; scrollTop: number },
): ScrollRestorePlan {
  // Hidden: the scrollTop read is meaningless (the browser zeroed it), so keep
  // whatever we last saw while visible.
  if (next.hidden) {
    return { state: { saved: prev.saved, hidden: true }, restoreTo: null };
  }

  // Becoming visible: put the remembered position back, unless there is nothing
  // to restore or it already matches (a redundant write would fire a pointless
  // scroll event).
  if (prev.hidden) {
    const needsRestore = prev.saved !== 0 && prev.saved !== next.scrollTop;
    return {
      state: { saved: prev.saved, hidden: false },
      restoreTo: needsRestore ? prev.saved : null,
    };
  }

  // Visible and staying visible: this is a real scroll position worth keeping,
  // including a scroll back to the top.
  return { state: { saved: next.scrollTop, hidden: false }, restoreTo: null };
}
