/**
 * Resizable layout columns (folder sidebar, email list): width limits and the
 * pointer/keyboard maths, kept pure so they are unit-tested without a DOM
 * (see `columnResize.test.ts`). `readingAreaRoom` is the one function that
 * reads the layout.
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

/** The narrowest a column may squeeze the reading area to: the floor the tag
 *  board already gives its reading pane (`min-w-[20rem]`). */
export const READING_AREA_MIN_WIDTH = 320;

/** Keep a width inside [spec.min, max]; `max` is narrower than `spec.max` when
 *  the reading area has little room left (see `maxWidthFor`). */
export function clampWidth(width: number, spec: ColumnSpec, max = spec.max): number {
  const w = Number.isFinite(width) ? width : spec.defaultWidth;
  return Math.round(Math.min(max, Math.max(spec.min, w)));
}

/**
 * The widest the column may become now: its width plus the room the reading
 * area has above its minimum, within the column's own maximum. With no room
 * left the column can still shrink, but not grow.
 */
export function maxWidthFor(width: number, room: number, spec: ColumnSpec): number {
  return Math.min(spec.max, width + Math.max(0, room));
}

/**
 * Pixels the reading area can give up before it drops below
 * `READING_AREA_MIN_WIDTH`: the reading pane (`data-reading-pane`) when the
 * main area shows one, otherwise the main area itself, whose view fills it.
 */
export function readingAreaRoom(main: HTMLElement | null): number {
  if (!main) return 0;
  const area = main.querySelector<HTMLElement>('[data-reading-pane]') ?? main;
  return area.clientWidth - READING_AREA_MIN_WIDTH;
}

/**
 * The widths to draw the columns at in `space`, the width the sidebar, the
 * list and the reading area share (`useColumnSpace`); `list` is null in a view
 * without a list column. The handles keep the reading area at its minimum
 * while the user resizes, but the space can shrink afterwards: the right dock
 * opens, the window narrows. A column widened past its default then gives the
 * extra width back, the list first, until the reading area has its minimum
 * again. Neither goes below its default, so a tight window looks as it did
 * before the columns could be resized; the saved widths stay, and come back
 * with the space. Null space (not measured yet) draws the saved widths.
 */
export function fitColumns(
  widths: { sidebar: number; list: number | null },
  space: number | null,
): { sidebar: number; list: number | null } {
  if (space === null) return widths;
  let excess = widths.sidebar + (widths.list ?? 0) + READING_AREA_MIN_WIDTH - Math.floor(space);
  const giveBack = (width: number, spec: ColumnSpec) => {
    const given = Math.min(Math.max(0, excess), Math.max(0, width - spec.defaultWidth));
    excess -= given;
    return width - given;
  };
  const list = widths.list === null ? null : giveBack(widths.list, LIST_COLUMN);
  return { sidebar: giveBack(widths.sidebar, SIDEBAR_COLUMN), list };
}

/** Parse a stored width; anything unusable means "use the default". */
export function parseWidth(raw: string, spec: ColumnSpec): number | null {
  const n = Number(raw);
  if (raw.trim() === '' || !Number.isFinite(n)) return null;
  return clampWidth(n, spec);
}

/** Width after dragging the handle `deltaX` pixels from where the drag started. */
export function dragWidth(startWidth: number, deltaX: number, spec: ColumnSpec, max = spec.max): number {
  return clampWidth(startWidth + deltaX, spec, max);
}

/**
 * Width after a key press on the handle, or null when the key does not resize.
 * ←/→ step, Shift+←/→ step ×10, Home/End jump to min/max (ARIA separator
 * conventions).
 */
export function keyWidth(width: number, key: string, shift: boolean, spec: ColumnSpec, max = spec.max): number | null {
  const step = shift ? KEY_STEP * 10 : KEY_STEP;
  switch (key) {
    case 'ArrowLeft':
      return clampWidth(width - step, spec, max);
    case 'ArrowRight':
      return clampWidth(width + step, spec, max);
    case 'Home':
      return clampWidth(spec.min, spec, max);
    case 'End':
      return clampWidth(spec.max, spec, max);
    default:
      return null;
  }
}
