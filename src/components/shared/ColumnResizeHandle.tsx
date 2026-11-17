import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { type ColumnSpec, clampWidth, dragWidth, keyWidth, maxWidthFor } from '@/lib/columnResize';
import { useOverlay } from '@/stores/overlayStore';

interface ColumnResizeHandleProps {
  spec: ColumnSpec;
  /** Saved width of the column on the handle's left. */
  width: number;
  /** Pixels the reading area can still give up before it reaches its minimum
   *  width (`readingAreaRoom`); read when a drag or a key press starts. */
  roomToGrow: () => number;
  /** Saves a width: once when a drag ends, at once for a key press or a double-click. */
  onCommit: (width: number) => void;
  /** Accessible name, e.g. "Resize folder list". */
  label: string;
  /** Hint shown on hover (drag / double-click to reset). */
  title: string;
}

/** Pixels the pointer travels before a press becomes a drag. A shorter press
 *  is a click, so both clicks of a double-click reach the handle. */
const DRAG_THRESHOLD = 3;

/**
 * A thin vertical splitter between two columns, placed right after the column
 * it resizes. Drag it with the mouse, use ←/→ (Shift for bigger steps) or
 * Home/End when it has focus, and double-click it to restore the default
 * width. Follows the WAI-ARIA "window splitter" pattern (role="separator").
 *
 * A drag sets the width on the column element itself (the handle's previous
 * sibling) and saves it once, on release: saving on every move wrote the
 * preference and re-rendered the whole app for each pixel.
 *
 * Once the pointer moves, a layer over the whole window keeps the pointer in
 * this document, as in `RightDock`: over an email iframe the window gets no
 * pointer events and the drag froze. The layer waits for the first pixels of
 * movement because a layer shown on press would take the release, and the
 * click and double-click would never reach the handle.
 */
export function ColumnResizeHandle({ spec, width, roomToGrow, onCommit, label, title }: ColumnResizeHandleProps) {
  const handleRef = useRef<HTMLDivElement>(null);
  const drag = useRef<{ startX: number; startWidth: number; max: number; width: number; moved: boolean } | null>(null);
  const [pressed, setPressed] = useState(false);
  // The column's width while a drag moves it (for aria-valuenow); null otherwise.
  const [liveWidth, setLiveWidth] = useState<number | null>(null);
  const dragging = liveWidth !== null;
  // The capture layer covers the whole window: no shortcut may act behind it.
  useOverlay(dragging);

  useEffect(() => {
    if (!pressed) return;
    const column = handleRef.current?.previousElementSibling;
    const onMove = (e: PointerEvent) => {
      const d = drag.current;
      if (!d) return;
      const deltaX = e.clientX - d.startX;
      if (!d.moved && Math.abs(deltaX) < DRAG_THRESHOLD) return;
      d.moved = true;
      e.preventDefault();
      d.width = dragWidth(d.startWidth, deltaX, spec, d.max);
      if (column instanceof HTMLElement) column.style.width = `${d.width}px`;
      setLiveWidth(d.width);
    };
    const stop = () => {
      const d = drag.current;
      drag.current = null;
      setPressed(false);
      setLiveWidth(null);
      document.body.style.userSelect = '';
      if (d?.moved) onCommit(d.width);
    };
    // Listen on window, not the handle: the pointer soon leaves a 6px target.
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
  }, [pressed, spec, onCommit]);

  return (
    <div
      ref={handleRef}
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={liveWidth ?? width}
      aria-valuemin={spec.min}
      aria-valuemax={spec.max}
      tabIndex={0}
      title={title}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        drag.current = {
          startX: e.clientX,
          startWidth: width,
          max: maxWidthFor(width, roomToGrow(), spec),
          width,
          moved: false,
        };
        // Keep the drag from selecting half the window's text.
        document.body.style.userSelect = 'none';
        setPressed(true);
      }}
      onDoubleClick={() => onCommit(clampWidth(spec.defaultWidth, spec, maxWidthFor(width, roomToGrow(), spec)))}
      onKeyDown={(e) => {
        const next = keyWidth(width, e.key, e.shiftKey, spec, maxWidthFor(width, roomToGrow(), spec));
        if (next === null) return;
        e.preventDefault();
        onCommit(next);
      }}
      // 6px hit area over the 1px border line; the line turns blue on hover,
      // while dragging, and when focused from the keyboard.
      className={`group relative z-20 -mx-[3px] w-[6px] flex-shrink-0 cursor-col-resize touch-none outline-none focus-visible:bg-primary-500/40 ${
        dragging ? 'bg-primary-500/40' : ''
      }`}
    >
      <span
        aria-hidden="true"
        className={`absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors ${
          dragging ? 'bg-primary-500' : 'bg-transparent group-hover:bg-primary-400'
        }`}
      />
      {dragging &&
        createPortal(
          <div data-testid="column-resize-drag-capture" className="fixed inset-0 z-[9999] cursor-col-resize" />,
          document.body,
        )}
    </div>
  );
}
