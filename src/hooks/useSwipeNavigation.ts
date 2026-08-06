import { useEffect, useRef } from 'react';
import { contentMayClaimDrag, recognizeSwipe, swipeNavAction } from '@/lib/swipeGesture';

export interface SwipeNavigationOptions {
  /** Only bind on the stacked (phone) layout — desktop keeps its buttons. */
  enabled: boolean;
  isSidebarOpen: boolean;
  /** Whether the current screen has somewhere to return to. */
  canGoBack: boolean;
  onBack: () => void;
  onCloseSidebar: () => void;
}

/**
 * Whether a touch that started on `target` at `startX` belongs to something
 * that owns horizontal drags already, and so must not also navigate.
 *
 * Two escapes, with different force:
 *
 * * `data-no-swipe` is absolute. It marks a surface that must never navigate
 *   (the reply compose, modals) at any x.
 * * An ancestor that *actually* scrolls horizontally — the calendar day strip,
 *   the settings tab rail, a wide table in an email — claims the drag only
 *   away from the screen edge (`contentMayClaimDrag`). At the edge, navigation
 *   wins, as it does in any iOS navigation stack.
 *
 * "Actually scrolls" matters twice over. `overflow-x-auto` on content that fits
 * does not scroll. And a vertical scroll container resolves its horizontal axis
 * to `auto` as well, so the tolerance is a few pixels rather than one: sub-pixel
 * layout rounding inside a thread must not read as a horizontal scroller.
 */
const SCROLL_OVERFLOW_TOLERANCE_PX = 8;

function ownsHorizontalDrag(target: EventTarget | null, startX: number): boolean {
  const contentMayClaim = contentMayClaimDrag(startX);
  let node = target instanceof Element ? target : null;
  while (node) {
    if (node.hasAttribute('data-no-swipe')) return true;
    if (contentMayClaim && node.scrollWidth > node.clientWidth + SCROLL_OVERFLOW_TOLERANCE_PX) {
      const overflowX = getComputedStyle(node).overflowX;
      if (overflowX === 'auto' || overflowX === 'scroll') return true;
    }
    node = node.parentElement;
  }
  return false;
}

/**
 * Swipe navigation for the stacked layout: a drag from the left edge goes
 * back, a leftward drag dismisses the open drawer.
 *
 * The decisions live in `lib/swipeGesture.ts`; this hook only feeds them live
 * touch coordinates. Listeners are bound to `window` (passive — the gesture
 * never blocks scrolling) and read their inputs through a ref, so a re-render
 * that changes `canGoBack` does not re-bind them mid-gesture.
 *
 * **Known gap:** email HTML renders in a sandboxed, null-origin iframe, and
 * touch events inside it never reach this listener. A swipe that starts over
 * message content is invisible here — which is why the thread's overflow menu
 * keeps an explicit "Back to inbox" item.
 */
export function useSwipeNavigation(options: SwipeNavigationOptions): void {
  const latest = useRef(options);
  latest.current = options;
  const start = useRef<{ x: number; y: number; time: number } | null>(null);
  const { enabled } = options;

  useEffect(() => {
    if (!enabled) return;

    const handleStart = (event: TouchEvent) => {
      // A second finger means a pinch or a two-finger scroll; neither is a
      // navigation gesture, and the first finger's path is now meaningless.
      if (event.touches.length !== 1) {
        start.current = null;
        return;
      }
      const touch = event.touches[0];
      if (ownsHorizontalDrag(event.target, touch.clientX)) {
        start.current = null;
        return;
      }
      start.current = { x: touch.clientX, y: touch.clientY, time: performance.now() };
    };

    const handleEnd = (event: TouchEvent) => {
      const began = start.current;
      start.current = null;
      const touch = event.changedTouches[0];
      if (!began || !touch) return;
      const direction = recognizeSwipe({
        startX: began.x,
        startY: began.y,
        endX: touch.clientX,
        endY: touch.clientY,
        elapsedMs: performance.now() - began.time,
      });
      const { isSidebarOpen, canGoBack, onBack, onCloseSidebar } = latest.current;
      switch (swipeNavAction(direction, { startX: began.x, isSidebarOpen, canGoBack })) {
        case 'goBack':
          onBack();
          break;
        case 'closeSidebar':
          onCloseSidebar();
          break;
        default:
          break;
      }
    };

    const handleCancel = () => {
      start.current = null;
    };

    window.addEventListener('touchstart', handleStart, { passive: true });
    window.addEventListener('touchend', handleEnd, { passive: true });
    window.addEventListener('touchcancel', handleCancel, { passive: true });
    return () => {
      window.removeEventListener('touchstart', handleStart);
      window.removeEventListener('touchend', handleEnd);
      window.removeEventListener('touchcancel', handleCancel);
    };
  }, [enabled]);
}
