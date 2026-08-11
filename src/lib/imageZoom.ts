// Zoom and pan arithmetic for the full-screen image viewer.
//
// The viewer draws one image centred on a full-screen surface, sized to fit,
// and then applies `translate(x, y) scale(scale)` to it. Scale 1 therefore
// means *fitted*, not natural size: the whole image is visible and there is
// nothing to pan to. Every rule about where the image may travel follows from
// that one convention.
//
// All of it is pure functions over plain numbers, so the thresholds and the
// focal-point algebra are table-testable without a touch device, a rendered
// component, or a DOM. The gesture handlers in `ZoomableImage` only supply the
// inputs — see `swipeGesture.ts` for the same split.

export interface ZoomState {
  scale: number;
  /** Screen-space translation in CSS px, measured from centred. */
  x: number;
  y: number;
}

export interface Size {
  width: number;
  height: number;
}

/** A point in viewport coordinates, with the origin at the viewport centre. */
export interface Point {
  x: number;
  y: number;
}

/** Fitted. Zooming out past this would leave the image adrift in empty space. */
export const MIN_SCALE = 1;

/** Far enough to read the small print on a scanned invoice. */
export const MAX_SCALE = 8;

/** Where a double-tap lands from fitted — close enough to read, far enough
 *  from the edges that the first pan still has room in both directions. */
export const DOUBLE_TAP_SCALE = 2.5;

export const FITTED: ZoomState = { scale: 1, x: 0, y: 0 };

export function clampScale(scale: number): number {
  if (Number.isNaN(scale)) return MIN_SCALE;
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

/**
 * The size the image is drawn at when fitted.
 *
 * Deliberately never larger than the image's own pixels: a 64px logo stretched
 * across a phone screen is a blurry mess, and the user can zoom in from fitted
 * if they want it bigger. A zero natural size means the image has not decoded
 * yet — fall back to the viewport so the first frame is not pinned at 0×0.
 */
export function fitSize(natural: Size, viewport: Size): Size {
  if (natural.width <= 0 || natural.height <= 0) return { ...viewport };
  const ratio = Math.min(viewport.width / natural.width, viewport.height / natural.height, 1);
  return { width: natural.width * ratio, height: natural.height * ratio };
}

/** Half the distance the content may travel before an edge enters the frame. */
function panLimit(contentSide: number, viewportSide: number, scale: number): number {
  return Math.max(0, (contentSide * scale - viewportSide) / 2);
}

/** Clamping a negative offset to a zero limit yields `-0`, which compares
 *  unequal to the fitted origin and leaks into the transform string. */
function clampAxis(offset: number, limit: number): number {
  const clamped = Math.min(limit, Math.max(-limit, offset));
  return clamped === 0 ? 0 : clamped;
}

/**
 * Keep the image over the viewport: no dragging it off into the void, and no
 * drift on an axis that still fits (a letterboxed panorama at 2x pans
 * horizontally only). At fitted this collapses to dead centre, which is what
 * makes zooming back out self-correcting.
 */
export function clampPan(state: ZoomState, content: Size, viewport: Size): ZoomState {
  const limitX = panLimit(content.width, viewport.width, state.scale);
  const limitY = panLimit(content.height, viewport.height, state.scale);
  return { scale: state.scale, x: clampAxis(state.x, limitX), y: clampAxis(state.y, limitY) };
}

/**
 * Scale by `factor` about `focal`, so the pixel under the finger stays under
 * the finger.
 *
 * A content point `c` sits on screen at `x + c * scale`. Holding that screen
 * position fixed across the scale change gives
 * `x' = focal - (focal - x) * scale'/scale`, which is all this is.
 */
export function zoomAt(state: ZoomState, factor: number, focal: Point, content: Size, viewport: Size): ZoomState {
  const scale = clampScale(state.scale * factor);
  const ratio = scale / state.scale;
  return clampPan(
    {
      scale,
      x: focal.x - (focal.x - state.x) * ratio,
      y: focal.y - (focal.y - state.y) * ratio,
    },
    content,
    viewport,
  );
}

export function panBy(state: ZoomState, delta: Point, content: Size, viewport: Size): ZoomState {
  return clampPan({ scale: state.scale, x: state.x + delta.x, y: state.y + delta.y }, content, viewport);
}

/** Double-tap: in to a readable scale around the tap, or all the way back out. */
export function toggleZoom(state: ZoomState, focal: Point, content: Size, viewport: Size): ZoomState {
  if (state.scale > MIN_SCALE) return FITTED;
  return zoomAt(state, DOUBLE_TAP_SCALE / state.scale, focal, content, viewport);
}

/**
 * A wheel/trackpad delta as a scale factor, matching the exponential curve the
 * email frame's own pinch handler uses. Capped per event because a trackpad can
 * deliver one enormous delta, and an uncapped flick slams straight to 8x.
 */
export function wheelZoomFactor(deltaY: number): number {
  return Math.min(2, Math.max(0.5, Math.exp(-deltaY * 0.01)));
}

export function distance(a: Point, b: Point): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

export function midpoint(a: Point, b: Point): Point {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
}
