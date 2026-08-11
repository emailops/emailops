import { describe, expect, it } from 'vitest';
import {
  clampPan,
  clampScale,
  DOUBLE_TAP_SCALE,
  distance,
  FITTED,
  fitSize,
  MAX_SCALE,
  MIN_SCALE,
  midpoint,
  panBy,
  toggleZoom,
  wheelZoomFactor,
  type ZoomState,
  zoomAt,
} from './imageZoom';

const VIEWPORT = { width: 500, height: 500 };
/** A square image fitted into the square viewport: scale 1 fills it exactly. */
const SQUARE = { width: 500, height: 500 };
/** A wide image fitted into the same viewport: letterboxed top and bottom. */
const WIDE = { width: 500, height: 200 };

/** Where a content-local point lands on screen under a given transform. */
function screenX(contentX: number, state: ZoomState): number {
  return state.x + contentX * state.scale;
}

describe('clampScale', () => {
  it('leaves a scale inside the range untouched', () => {
    expect(clampScale(2.5)).toBe(2.5);
  });

  it('clamps below fitted and above the maximum', () => {
    expect(clampScale(0.1)).toBe(MIN_SCALE);
    expect(clampScale(1000)).toBe(MAX_SCALE);
  });

  it('falls back to fitted for a non-finite scale', () => {
    expect(clampScale(Number.NaN)).toBe(MIN_SCALE);
    expect(clampScale(Number.POSITIVE_INFINITY)).toBe(MAX_SCALE);
  });
});

describe('fitSize', () => {
  it('shrinks an oversized image to fit inside the viewport, preserving aspect', () => {
    expect(fitSize({ width: 2000, height: 1000 }, VIEWPORT)).toEqual({ width: 500, height: 250 });
  });

  it('fits by the constraining axis', () => {
    expect(fitSize({ width: 1000, height: 4000 }, VIEWPORT)).toEqual({ width: 125, height: 500 });
  });

  // A 64px logo blown up to fill a phone screen is a blurry mess. Fitted means
  // "no larger than natural"; the user can still zoom in from there.
  it('never upscales an image smaller than the viewport', () => {
    expect(fitSize({ width: 64, height: 32 }, VIEWPORT)).toEqual({ width: 64, height: 32 });
  });

  it('falls back to the viewport when the natural size is unknown', () => {
    expect(fitSize({ width: 0, height: 0 }, VIEWPORT)).toEqual(VIEWPORT);
  });
});

describe('zoomAt', () => {
  it('keeps the content point under the focal point pinned to it', () => {
    const focal = { x: 100, y: 0 };
    const next = zoomAt(FITTED, 2, focal, SQUARE, VIEWPORT);
    expect(next.scale).toBe(2);
    // The content point that was under the finger is still under the finger.
    const contentPoint = (focal.x - FITTED.x) / FITTED.scale;
    expect(screenX(contentPoint, next)).toBeCloseTo(focal.x);
  });

  it('zooming at the centre does not translate', () => {
    expect(zoomAt(FITTED, 3, { x: 0, y: 0 }, SQUARE, VIEWPORT)).toEqual({ scale: 3, x: 0, y: 0 });
  });

  it('clamps the scale to the maximum', () => {
    expect(zoomAt(FITTED, 100, { x: 0, y: 0 }, SQUARE, VIEWPORT).scale).toBe(MAX_SCALE);
  });

  it('cannot zoom out below fitted', () => {
    expect(zoomAt(FITTED, 0.25, { x: 0, y: 0 }, SQUARE, VIEWPORT)).toEqual(FITTED);
  });

  // Zooming out re-centres on its own: the pan clamp shrinks with the scale, so
  // returning to fitted always lands back at the origin with no stray offset.
  it('re-centres when zooming back out to fitted', () => {
    const zoomed = zoomAt(FITTED, 4, { x: 200, y: 200 }, SQUARE, VIEWPORT);
    expect(zoomed.x).not.toBe(0);
    expect(zoomAt(zoomed, 0.25, { x: 200, y: 200 }, SQUARE, VIEWPORT)).toEqual(FITTED);
  });
});

describe('clampPan', () => {
  it('pins a fitted image to the centre — there is nothing to pan to', () => {
    expect(clampPan({ scale: 1, x: 200, y: -80 }, SQUARE, VIEWPORT)).toEqual(FITTED);
  });

  it('allows travel up to half the overflow', () => {
    // At 2x a 500px-wide image is 1000px on a 500px viewport: 250px each way.
    expect(clampPan({ scale: 2, x: 200, y: 0 }, SQUARE, VIEWPORT).x).toBe(200);
    expect(clampPan({ scale: 2, x: 400, y: 0 }, SQUARE, VIEWPORT).x).toBe(250);
    expect(clampPan({ scale: 2, x: -400, y: 0 }, SQUARE, VIEWPORT).x).toBe(-250);
  });

  // Each axis is clamped by its own overflow. A letterboxed wide image must not
  // drift vertically at 2x just because it may drift horizontally.
  it('locks an axis that still fits inside the viewport', () => {
    const panned = clampPan({ scale: 2, x: 100, y: 100 }, WIDE, VIEWPORT);
    expect(panned.x).toBe(100);
    expect(panned.y).toBe(0);
  });
});

describe('panBy', () => {
  it('accumulates a drag', () => {
    const start = { scale: 2, x: 0, y: 0 };
    expect(panBy(start, { x: 30, y: -20 }, SQUARE, VIEWPORT)).toEqual({ scale: 2, x: 30, y: -20 });
  });

  it('stops at the edge instead of dragging the image off screen', () => {
    const start = { scale: 2, x: 240, y: 0 };
    expect(panBy(start, { x: 500, y: 0 }, SQUARE, VIEWPORT).x).toBe(250);
  });
});

describe('toggleZoom', () => {
  it('jumps to the double-tap scale around the tapped point when fitted', () => {
    const focal = { x: 120, y: -40 };
    const next = toggleZoom(FITTED, focal, SQUARE, VIEWPORT);
    expect(next.scale).toBe(DOUBLE_TAP_SCALE);
    expect(screenX(focal.x, next)).toBeCloseTo(focal.x);
  });

  it('returns to fitted from any zoomed state', () => {
    expect(toggleZoom({ scale: 4, x: 100, y: 100 }, { x: 0, y: 0 }, SQUARE, VIEWPORT)).toEqual(FITTED);
  });
});

describe('wheelZoomFactor', () => {
  it('scrolling up zooms in and scrolling down zooms out', () => {
    expect(wheelZoomFactor(-100)).toBeGreaterThan(1);
    expect(wheelZoomFactor(100)).toBeLessThan(1);
    expect(wheelZoomFactor(0)).toBe(1);
  });

  // A trackpad can deliver a single enormous delta; without a cap one flick
  // slams the image to 8x.
  it('caps how much a single event can change the scale', () => {
    expect(wheelZoomFactor(-100000)).toBeLessThanOrEqual(2);
    expect(wheelZoomFactor(100000)).toBeGreaterThanOrEqual(0.5);
  });
});

describe('touch helpers', () => {
  it('measures the distance between two touch points', () => {
    expect(distance({ x: 0, y: 0 }, { x: 3, y: 4 })).toBe(5);
  });

  it('finds the midpoint between two touch points', () => {
    expect(midpoint({ x: 0, y: 0 }, { x: 10, y: 20 })).toEqual({ x: 5, y: 10 });
  });
});
