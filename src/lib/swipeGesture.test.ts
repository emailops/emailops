import { describe, expect, it } from 'vitest';
import {
  contentMayClaimDrag,
  EDGE_START_MAX_PX,
  recognizeSwipe,
  type SwipeSample,
  swipeNavAction,
} from './swipeGesture';

/** A clean, fast, horizontal drag of `dx` px starting at `startX`. */
const drag = (dx: number, dy = 0, elapsedMs = 200, startX = 10): SwipeSample => ({
  startX,
  startY: 100,
  endX: startX + dx,
  endY: 100 + dy,
  elapsedMs,
});

describe('recognizeSwipe', () => {
  it('recognizes a decisive drag in each direction', () => {
    expect(recognizeSwipe(drag(120))).toBe('right');
    expect(recognizeSwipe(drag(-120))).toBe('left');
  });

  it('ignores a drag too short to be deliberate', () => {
    // A tap wanders a few pixels. Acting on that would make every tap on a
    // message a coin flip between opening it and leaving the screen.
    expect(recognizeSwipe(drag(12))).toBeNull();
    expect(recognizeSwipe(drag(-12))).toBeNull();
  });

  it('ignores a drag that is mostly vertical', () => {
    // Scrolling a long thread is never a perfectly straight line; the
    // horizontal component must dominate before this counts as a swipe.
    expect(recognizeSwipe(drag(80, 200))).toBeNull();
    expect(recognizeSwipe(drag(-80, -200))).toBeNull();
  });

  it('accepts a long drag with a little vertical drift', () => {
    expect(recognizeSwipe(drag(150, 30))).toBe('right');
  });

  it('ignores a slow drag', () => {
    // Holding and dragging over a second is a selection or a scroll that
    // changed its mind, not a flick.
    expect(recognizeSwipe(drag(200, 0, 2000))).toBeNull();
  });

  it('is unaffected by where on the screen the drag happened', () => {
    // The edge rule belongs to the navigation decision, not to recognition.
    expect(recognizeSwipe(drag(120, 0, 200, 300))).toBe('right');
  });
});

describe('contentMayClaimDrag', () => {
  it('gives the screen edge to navigation', () => {
    // Regression: opening a message killed the back gesture. The thread scrolls
    // with `overflow-y-auto`, CSS resolves the other axis of a scroll container
    // to `auto` too, and one email a few pixels too wide made the whole thread
    // read as a horizontal scroller that owned every drag.
    expect(contentMayClaimDrag(0)).toBe(false);
    expect(contentMayClaimDrag(EDGE_START_MAX_PX)).toBe(false);
  });

  it('leaves mid-screen drags to whatever is under the finger', () => {
    expect(contentMayClaimDrag(EDGE_START_MAX_PX + 1)).toBe(true);
    expect(contentMayClaimDrag(300)).toBe(true);
  });
});

describe('swipeNavAction', () => {
  const ctx = (over: Partial<Parameters<typeof swipeNavAction>[1]> = {}) => ({
    startX: 10,
    isSidebarOpen: false,
    canGoBack: true,
    ...over,
  });

  it('goes back on a rightward swipe from the left edge', () => {
    expect(swipeNavAction('right', ctx())).toBe('goBack');
  });

  it('ignores a rightward swipe that started away from the edge', () => {
    // Mid-screen horizontal drags belong to whatever is under the finger —
    // a horizontally scrolling day strip, a wide table inside an email.
    expect(swipeNavAction('right', ctx({ startX: EDGE_START_MAX_PX + 1 }))).toBe('none');
  });

  it('does nothing when there is nowhere to go back to', () => {
    // Calendar, dashboard and a bare inbox list are all terminal screens.
    expect(swipeNavAction('right', ctx({ canGoBack: false }))).toBe('none');
  });

  it('closes the drawer on a leftward swipe while it is open', () => {
    expect(swipeNavAction('left', ctx({ isSidebarOpen: true }))).toBe('closeSidebar');
  });

  it('leaves the drawer alone on a rightward swipe', () => {
    // It is already open; going "back" underneath it would change a screen
    // the user cannot see.
    expect(swipeNavAction('right', ctx({ isSidebarOpen: true }))).toBe('none');
  });

  it('reserves the leftward swipe when the drawer is closed', () => {
    // Deliberately unassigned for now, so a future "next message" gesture is
    // free to take it.
    expect(swipeNavAction('left', ctx())).toBe('none');
  });

  it('does nothing when no swipe was recognized', () => {
    expect(swipeNavAction(null, ctx())).toBe('none');
    expect(swipeNavAction(null, ctx({ isSidebarOpen: true }))).toBe('none');
  });
});
