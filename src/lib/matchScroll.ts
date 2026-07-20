// Scroll-positioning helpers for the in-thread search. The email body renders
// in a sandboxed null-origin iframe, so scrollIntoView from inside the frame
// cannot scroll the surrounding thread container — instead the bridge reports
// the active match's offset within the frame and the parent computes where to
// scroll the container.

export interface ScrollContainerMetrics {
  scrollTop: number;
  /** getBoundingClientRect().top of the scroll container. */
  rectTop: number;
  clientHeight: number;
}

/**
 * Target scrollTop that places a match (at `matchTop` px inside a frame whose
 * viewport-relative top is `frameRectTop`) a third of the way down the
 * container, clamped to non-negative.
 */
export function computeMatchScrollTop(
  container: ScrollContainerMetrics,
  frameRectTop: number,
  matchTop: number,
): number {
  return Math.max(0, container.scrollTop + (frameRectTop - container.rectTop) + matchTop - container.clientHeight / 3);
}

/** Nearest ancestor with a scrollable overflow-y, or null. */
export function findScrollParent(el: HTMLElement): HTMLElement | null {
  let node = el.parentElement;
  while (node) {
    const overflowY = getComputedStyle(node).overflowY;
    if (overflowY === 'auto' || overflowY === 'scroll') return node;
    node = node.parentElement;
  }
  return null;
}
