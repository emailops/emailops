/**
 * Resizable layout columns (folder sidebar, email list): width limits and the
 * pointer/keyboard maths, kept pure so they are unit-tested without a DOM
 * (see `columnResize.test.ts`).
 */

export interface ColumnSpec {
  /** Preference key the width is stored under. */
  prefKey: string;
  /** Width the app shipped with — also what a double-click restores. */
  defaultWidth: number;
  min: number;
  max: number;
}

/** Folder sidebar: was a fixed `w-64` (256px). */
export const SIDEBAR_COLUMN: ColumnSpec = {
  prefKey: 'layout.sidebar_width',
  defaultWidth: 256,
  min: 180,
  max: 420,
};

/** Email list in the split layout: was a fixed `w-96` (384px). */
export const LIST_COLUMN: ColumnSpec = {
  prefKey: 'layout.list_width',
  defaultWidth: 384,
  min: 300,
  max: 800,
};

/** Pixels per arrow-key press; Shift moves ten times faster. */
export const KEY_STEP = 16;

export function clampWidth(width: number, spec: ColumnSpec): number {
  if (!Number.isFinite(width)) return spec.defaultWidth;
  return Math.round(Math.min(spec.max, Math.max(spec.min, width)));
}

/** Parse a stored width; anything unusable means "use the default". */
export function parseWidth(raw: string, spec: ColumnSpec): number | null {
  const n = Number(raw);
  if (raw.trim() === '' || !Number.isFinite(n)) return null;
  return clampWidth(n, spec);
}

/** Width after dragging the handle `deltaX` pixels from where the drag started. */
export function dragWidth(startWidth: number, deltaX: number, spec: ColumnSpec): number {
  return clampWidth(startWidth + deltaX, spec);
}

/**
 * Width after a key press on the handle, or null when the key does not resize.
 * ←/→ step, Shift+←/→ step ×10, Home/End jump to min/max (ARIA separator
 * conventions).
 */
export function keyWidth(width: number, key: string, shift: boolean, spec: ColumnSpec): number | null {
  const step = shift ? KEY_STEP * 10 : KEY_STEP;
  switch (key) {
    case 'ArrowLeft':
      return clampWidth(width - step, spec);
    case 'ArrowRight':
      return clampWidth(width + step, spec);
    case 'Home':
      return spec.min;
    case 'End':
      return spec.max;
    default:
      return null;
  }
}
