// One image, pannable and zoomable by pinch, wheel, drag and double-tap.
//
// All the arithmetic — focal-point zoom, pan limits, the fitted size — lives in
// `lib/imageZoom.ts` and is unit-tested there. What is left here is the adapter:
// turn pointers and wheels into (factor, focal, delta) and hand them over.
//
// Pointer events rather than touch events, for two reasons: one code path
// covers a finger, a pen and a mouse, and `touch-action: none` on the surface
// tells the browser we own the gesture, so nothing has to be cancelled in a
// listener React would have attached as passive anyway.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  distance,
  FITTED,
  fitSize,
  midpoint,
  type Point,
  panBy,
  type Size,
  toggleZoom,
  wheelZoomFactor,
  type ZoomState,
  zoomAt,
} from '@/lib/imageZoom';

export interface ZoomableImageProps {
  src: string;
  alt: string;
  /** Applied to the surface the image is centred in. */
  className?: string;
  /** Called when the image cannot be decoded. */
  onError?: () => void;
}

/** A tap has to be brief and still to count as one half of a double-tap. */
const TAP_MAX_MS = 300;
const TAP_MAX_MOVE_PX = 12;
/** How long the two taps may be apart. Matches the platform double-tap window. */
const DOUBLE_TAP_MAX_GAP_MS = 320;

interface TapRecord {
  x: number;
  y: number;
  time: number;
}

/** "Not measured yet", for both the surface and the image. */
const UNKNOWN: Size = { width: 0, height: 0 };

export function ZoomableImage({ src, alt, className, onError }: ZoomableImageProps) {
  const surfaceRef = useRef<HTMLDivElement>(null);
  const imageRef = useRef<HTMLImageElement>(null);
  const [viewport, setViewport] = useState<Size>(UNKNOWN);
  const [natural, setNatural] = useState<Size>(UNKNOWN);
  const [zoom, setZoom] = useState<ZoomState>(FITTED);
  const [gesturing, setGesturing] = useState(false);

  const content = useMemo(() => fitSize(natural, viewport), [natural, viewport]);

  // Gesture bookkeeping is deliberately in refs: a pinch updates several times
  // per frame and none of it should cost a render on its own.
  const pointers = useRef(new Map<number, Point>());
  const pinch = useRef<{ distance: number; scale: number } | null>(null);
  const pressStart = useRef<TapRecord | null>(null);
  const lastTap = useRef<TapRecord | null>(null);
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const geometry = useRef({ content, viewport });
  geometry.current = { content, viewport };

  // A different image starts fitted, with its size unknown again until it
  // decodes. Reset *during* the render that first sees the new src, not from an
  // effect: effects run after paint, and a `data:` image — which every inline
  // email image is — can finish decoding in between. An effect-based reset then
  // lands after the size arrives and wipes it, leaving the picture stretched to
  // the shape of the screen for as long as it is open.
  const [renderedSrc, setRenderedSrc] = useState(src);
  if (renderedSrc !== src) {
    setRenderedSrc(src);
    setNatural(UNKNOWN);
    setZoom(FITTED);
  }

  const readNaturalSize = useCallback(() => {
    const img = imageRef.current;
    if (!img?.naturalWidth || !img.naturalHeight) return;
    setNatural({ width: img.naturalWidth, height: img.naturalHeight });
  }, []);

  // Same race from the other side: a `data:` image can be `complete` before
  // React attaches `onLoad`, so that event never arrives and the size has to be
  // read off the element instead.
  useEffect(readNaturalSize, [readNaturalSize, renderedSrc]);

  useEffect(() => {
    const el = surfaceRef.current;
    if (!el) return;
    const measure = () => setViewport({ width: el.clientWidth, height: el.clientHeight });
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  /** Viewport coordinates with the origin at the centre, which is where the
   *  transform's own origin sits. */
  const toFocal = useCallback((clientX: number, clientY: number): Point => {
    const rect = surfaceRef.current?.getBoundingClientRect();
    if (!rect) return { x: 0, y: 0 };
    return { x: clientX - (rect.left + rect.width / 2), y: clientY - (rect.top + rect.height / 2) };
  }, []);

  // Registered by hand because React attaches `wheel` passively at the root, so
  // a preventDefault from an onWheel prop would be ignored — and without it the
  // pinch on a trackpad zooms the whole app instead of the picture.
  useEffect(() => {
    const el = surfaceRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const { content: c, viewport: v } = geometry.current;
      setZoom((z) => zoomAt(z, wheelZoomFactor(e.deltaY), toFocal(e.clientX, e.clientY), c, v));
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [toFocal]);

  const handlePointerDown = useCallback((e: React.PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture?.(e.pointerId);
    pointers.current.set(e.pointerId, { x: e.clientX, y: e.clientY });
    setGesturing(true);
    if (pointers.current.size === 1) {
      pressStart.current = { x: e.clientX, y: e.clientY, time: Date.now() };
    } else if (pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      pinch.current = { distance: distance(a, b), scale: zoomRef.current.scale };
      // Two fingers down is a pinch, not a tap that happened to be near another.
      pressStart.current = null;
    }
  }, []);

  const handlePointerMove = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      const previous = pointers.current.get(e.pointerId);
      if (!previous) return;
      const current = { x: e.clientX, y: e.clientY };
      pointers.current.set(e.pointerId, current);
      const points = [...pointers.current.values()];
      const { content: c, viewport: v } = geometry.current;

      if (points.length >= 2 && pinch.current) {
        // The scale is absolute against the spread the fingers started at, so a
        // pinch that returns to where it began returns the image with it —
        // accumulating per-move factors would drift instead.
        const [a, b] = points;
        const base = pinch.current;
        const target = base.scale * (distance(a, b) / base.distance);
        const focal = toFocal(midpoint(a, b).x, midpoint(a, b).y);
        setZoom((z) => zoomAt(z, target / z.scale, focal, c, v));
        return;
      }

      // A drag at fitted scale has nowhere to go: `panBy` clamps it to centre.
      setZoom((z) => panBy(z, { x: current.x - previous.x, y: current.y - previous.y }, c, v));
    },
    [toFocal],
  );

  const endPointer = useCallback(
    (e: React.PointerEvent<HTMLDivElement>) => {
      pointers.current.delete(e.pointerId);
      if (pointers.current.size < 2) pinch.current = null;
      if (pointers.current.size === 0) setGesturing(false);

      const press = pressStart.current;
      pressStart.current = null;
      if (!press) return;
      const now = Date.now();
      const moved = distance(press, { x: e.clientX, y: e.clientY });
      if (now - press.time > TAP_MAX_MS || moved > TAP_MAX_MOVE_PX) return;

      const previous = lastTap.current;
      const isDouble =
        previous !== null &&
        now - previous.time <= DOUBLE_TAP_MAX_GAP_MS &&
        distance(previous, { x: e.clientX, y: e.clientY }) <= TAP_MAX_MOVE_PX * 3;
      if (isDouble) {
        lastTap.current = null;
        const { content: c, viewport: v } = geometry.current;
        const focal = toFocal(e.clientX, e.clientY);
        setZoom((z) => toggleZoom(z, focal, c, v));
        return;
      }
      lastTap.current = { x: e.clientX, y: e.clientY, time: now };
    },
    [toFocal],
  );

  const canPan = zoom.scale > 1;

  return (
    <div
      ref={surfaceRef}
      className={`relative flex items-center justify-center overflow-hidden select-none ${className ?? ''}`}
      style={{ touchAction: 'none', cursor: canPan ? (gesturing ? 'grabbing' : 'grab') : 'default' }}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endPointer}
      onPointerCancel={endPointer}
    >
      <img
        ref={imageRef}
        src={src}
        alt={alt}
        draggable={false}
        onLoad={readNaturalSize}
        onError={onError}
        style={{
          // Before the surface is measured, `fitSize` has nothing to work with;
          // the max-* pair keeps that first frame inside the screen.
          width: content.width || undefined,
          height: content.height || undefined,
          maxWidth: '100%',
          maxHeight: '100%',
          transform: `translate(${zoom.x}px, ${zoom.y}px) scale(${zoom.scale})`,
          // Animate the jump a double-tap makes, but never a live drag, which
          // would trail a frame behind the finger.
          transition: gesturing ? 'none' : 'transform 150ms ease-out',
          willChange: 'transform',
        }}
      />
    </div>
  );
}
