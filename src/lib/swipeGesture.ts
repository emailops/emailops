// Touch-swipe recognition and what a recognized swipe means for navigation.
//
// Both decisions are pure functions over plain numbers so the thresholds are
// table-testable without a DOM, a touch device, or a rendered component. The
// hook (`hooks/useSwipeNavigation.ts`) only supplies the inputs.

/** One completed touch drag, in client coordinates. */
export interface SwipeSample {
  startX: number;
  startY: number;
  endX: number;
  endY: number;
  /** Milliseconds between touchstart and touchend. */
  elapsedMs: number;
}

export type SwipeDirection = 'left' | 'right';

/** Below this the drag is indistinguishable from the wander of a tap. */
const MIN_DISTANCE_PX = 60;

/** How much longer the horizontal component must be than the vertical one.
 *  Vertical scrolling in a long thread is never perfectly straight, so this
 *  is what keeps a scroll from reading as a swipe. */
const MIN_AXIS_RATIO = 1.5;

/** A drag slower than this is a scroll or a text selection, not a flick. */
const MAX_DURATION_MS = 800;

/** How far from the left edge a back-swipe may start. Matches the width iOS
 *  itself reserves for its interactive pop gesture. */
export const EDGE_START_MAX_PX = 40;

/**
 * The direction of a deliberate horizontal swipe, or `null` if this drag was
 * a tap, a scroll, or too slow to be a flick.
 */
export function recognizeSwipe(sample: SwipeSample): SwipeDirection | null {
  const dx = sample.endX - sample.startX;
  const dy = sample.endY - sample.startY;
  if (sample.elapsedMs > MAX_DURATION_MS) return null;
  if (Math.abs(dx) < MIN_DISTANCE_PX) return null;
  if (Math.abs(dx) < Math.abs(dy) * MIN_AXIS_RATIO) return null;
  return dx > 0 ? 'right' : 'left';
}

/**
 * Whether content under the finger is allowed to claim a horizontal drag.
 *
 * False at the screen edge: the edge belongs to navigation, the way it does in
 * every iOS app with a navigation stack. Without this exception the back
 * gesture dies inside a thread — the message list scrolls with
 * `overflow-y-auto`, and CSS resolves the *other* axis of a scroll container to
 * `auto` too, so a message a few pixels wider than the viewport makes the whole
 * thread look like a horizontal scroller and swallows the swipe.
 *
 * An explicit `data-no-swipe` marker is NOT subject to this: surfaces that opt
 * out (the reply compose, modals) mean it at every x.
 */
export function contentMayClaimDrag(startX: number): boolean {
  return startX > EDGE_START_MAX_PX;
}

/** What a recognized swipe should do to the current screen. */
export type SwipeNavAction = 'goBack' | 'closeSidebar' | 'none';

export interface SwipeNavContext {
  /** Where the drag began, so a back-swipe can require the left edge. */
  startX: number;
  isSidebarOpen: boolean;
  /** Whether the current screen has somewhere to return to. */
  canGoBack: boolean;
}

/**
 * Map a swipe onto a navigation action.
 *
 * Back is a **rightward** drag from the left edge — the platform gesture iOS
 * users already have in their fingers, and the direction the screen visually
 * moves. Leftward is deliberately reserved (except for dismissing the open
 * drawer, which slid in from the left and so goes back out the same way), so a
 * later "next message" gesture can claim it.
 *
 * The edge requirement is what keeps this from stealing horizontal drags that
 * belong to the content: the calendar's day strip, the settings tab rail, a
 * wide table inside an email.
 */
export function swipeNavAction(direction: SwipeDirection | null, ctx: SwipeNavContext): SwipeNavAction {
  if (direction === null) return 'none';
  if (ctx.isSidebarOpen) return direction === 'left' ? 'closeSidebar' : 'none';
  if (direction === 'right' && ctx.canGoBack && ctx.startX <= EDGE_START_MAX_PX) return 'goBack';
  return 'none';
}

/** Where a back gesture should land, given what is currently on screen. */
export type BackTarget =
  /** A message is open: close it and return to the list it came from. */
  | 'closeThread'
  /** Leave this view for whichever one the user came from. */
  | 'previousView'
  /** Already at the root. */
  | 'none';

export interface BackContext {
  viewMode: string;
  /** A message (not a compose tab) is open in an email list view. */
  isThreadOpen: boolean;
}

/**
 * Unwind one level of navigation.
 *
 * "Back" means the view the user came from, not a view-internal pane. Chat is
 * deliberately NOT special: swiping back out of a conversation returns to
 * wherever chat was opened from, rather than to the conversation list. The
 * history is reachable from inside chat; the gesture is for leaving.
 *
 * An open message is the one exception, because it genuinely sits on top of the
 * list it was opened from — closing it is a step, not a departure.
 *
 * The root view returns `none`: the gesture is inert there rather than
 * inventing a destination.
 */
export function planBackTarget(ctx: BackContext): BackTarget {
  if (ctx.isThreadOpen) return 'closeThread';
  return ctx.viewMode === 'inbox' ? 'none' : 'previousView';
}
