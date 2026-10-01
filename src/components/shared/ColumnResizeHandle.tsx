import { useRef, useState } from 'react';
import { type ColumnSpec, dragWidth, keyWidth } from '@/lib/columnResize';

interface ColumnResizeHandleProps {
  spec: ColumnSpec;
  /** Current width of the column on the handle's left. */
  width: number;
  /** Live updates while dragging or pressing keys. */
  onResize: (width: number) => void;
  /** Accessible name, e.g. "Resize folder list". */
  label: string;
  /** Hint shown on hover (drag / double-click to reset). */
  title: string;
}

/**
 * A thin vertical splitter between two columns, placed right after the column
 * it resizes. Drag it with the mouse, use ←/→ (Shift for bigger steps) or
 * Home/End when it has focus, and double-click it to restore the default
 * width. Follows the WAI-ARIA "window splitter" pattern (role="separator").
 *
 * Pointer capture keeps the drag going even when the pointer leaves the thin
 * handle (or crosses an email iframe), and text selection is suppressed for
 * the duration so dragging does not highlight half the window.
 */
export function ColumnResizeHandle({ spec, width, onResize, label, title }: ColumnResizeHandleProps) {
  const drag = useRef<{ startX: number; startWidth: number } | null>(null);
  const [active, setActive] = useState(false);

  const end = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return;
    drag.current = null;
    setActive(false);
    document.body.style.userSelect = '';
    if (e.currentTarget.hasPointerCapture?.(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={spec.min}
      aria-valuemax={spec.max}
      tabIndex={0}
      title={title}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        drag.current = { startX: e.clientX, startWidth: width };
        setActive(true);
        document.body.style.userSelect = 'none';
        e.currentTarget.setPointerCapture?.(e.pointerId);
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        onResize(dragWidth(drag.current.startWidth, e.clientX - drag.current.startX, spec));
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onDoubleClick={() => onResize(spec.defaultWidth)}
      onKeyDown={(e) => {
        const next = keyWidth(width, e.key, e.shiftKey, spec);
        if (next === null) return;
        e.preventDefault();
        onResize(next);
      }}
      // 6px hit area over the 1px border line; the line turns blue on hover,
      // while dragging, and when focused from the keyboard.
      className={`group relative z-20 -mx-[3px] w-[6px] flex-shrink-0 cursor-col-resize touch-none outline-none focus-visible:bg-primary-500/40 ${
        active ? 'bg-primary-500/40' : ''
      }`}
    >
      <span
        aria-hidden="true"
        className={`absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors ${
          active ? 'bg-primary-500' : 'bg-transparent group-hover:bg-primary-400'
        }`}
      />
    </div>
  );
}
