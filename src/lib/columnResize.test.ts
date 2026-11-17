import { describe, expect, it } from 'vitest';
import {
  clampWidth,
  dragWidth,
  KEY_STEP,
  keyWidth,
  LIST_COLUMN,
  maxWidthFor,
  parseWidth,
  READING_AREA_MIN_WIDTH,
  readingAreaRoom,
  SIDEBAR_COLUMN,
} from './columnResize';

describe('column specs', () => {
  it('default to the widths the app used before (w-64 / w-96)', () => {
    expect(SIDEBAR_COLUMN.defaultWidth).toBe(256);
    expect(LIST_COLUMN.defaultWidth).toBe(384);
  });

  it('keep the default inside the allowed range', () => {
    for (const spec of [SIDEBAR_COLUMN, LIST_COLUMN]) {
      expect(spec.defaultWidth).toBeGreaterThanOrEqual(spec.min);
      expect(spec.defaultWidth).toBeLessThanOrEqual(spec.max);
    }
  });
});

describe('clampWidth / parseWidth', () => {
  it('clamps into [min, max] and rounds', () => {
    expect(clampWidth(10, SIDEBAR_COLUMN)).toBe(SIDEBAR_COLUMN.min);
    expect(clampWidth(9999, SIDEBAR_COLUMN)).toBe(SIDEBAR_COLUMN.max);
    expect(clampWidth(300.6, SIDEBAR_COLUMN)).toBe(301);
    expect(clampWidth(Number.NaN, LIST_COLUMN)).toBe(LIST_COLUMN.defaultWidth);
  });

  it('reads a stored width, clamped, and rejects junk', () => {
    expect(parseWidth('500', LIST_COLUMN)).toBe(500);
    expect(parseWidth('5000', LIST_COLUMN)).toBe(LIST_COLUMN.max);
    expect(parseWidth('', LIST_COLUMN)).toBeNull();
    expect(parseWidth('wide', LIST_COLUMN)).toBeNull();
  });
});

describe('dragWidth', () => {
  it('follows the pointer, within limits', () => {
    expect(dragWidth(256, 40, SIDEBAR_COLUMN)).toBe(296);
    expect(dragWidth(256, -40, SIDEBAR_COLUMN)).toBe(216);
    expect(dragWidth(256, -500, SIDEBAR_COLUMN)).toBe(SIDEBAR_COLUMN.min);
  });

  it('stops at a narrower limit when one is given', () => {
    expect(dragWidth(384, 400, LIST_COLUMN, 500)).toBe(500);
  });
});

describe('keyWidth', () => {
  it('steps with the arrows, faster with Shift, and jumps with Home/End', () => {
    expect(keyWidth(384, 'ArrowRight', false, LIST_COLUMN)).toBe(384 + KEY_STEP);
    expect(keyWidth(384, 'ArrowLeft', false, LIST_COLUMN)).toBe(384 - KEY_STEP);
    expect(keyWidth(384, 'ArrowRight', true, LIST_COLUMN)).toBe(384 + KEY_STEP * 10);
    expect(keyWidth(384, 'Home', false, LIST_COLUMN)).toBe(LIST_COLUMN.min);
    expect(keyWidth(384, 'End', false, LIST_COLUMN)).toBe(LIST_COLUMN.max);
  });

  // Home/End go through the same clamp as the arrows, so a narrower limit
  // (little room left for the reading area) holds for every key.
  it('keeps every key inside a narrower limit', () => {
    expect(keyWidth(384, 'End', false, LIST_COLUMN, 500)).toBe(500);
    expect(keyWidth(384, 'ArrowRight', true, LIST_COLUMN, 500)).toBe(500);
    expect(keyWidth(384, 'Home', false, LIST_COLUMN, 500)).toBe(LIST_COLUMN.min);
  });

  it('ignores other keys', () => {
    expect(keyWidth(384, 'Enter', false, LIST_COLUMN)).toBeNull();
    expect(keyWidth(384, 'ArrowUp', false, LIST_COLUMN)).toBeNull();
  });
});

describe('maxWidthFor', () => {
  it('lets the column grow by the room the reading area has above its minimum', () => {
    expect(maxWidthFor(384, 100, LIST_COLUMN)).toBe(484);
  });

  it('never goes past the column maximum', () => {
    expect(maxWidthFor(384, 5000, LIST_COLUMN)).toBe(LIST_COLUMN.max);
  });

  // The reading area is already at or under its minimum (a narrow window):
  // the column may still shrink, but not grow.
  it('holds the column at its width when the reading area has no room', () => {
    expect(maxWidthFor(384, -50, LIST_COLUMN)).toBe(384);
  });
});

describe('readingAreaRoom', () => {
  function sized<T extends HTMLElement>(el: T, width: number): T {
    Object.defineProperty(el, 'clientWidth', { configurable: true, value: width });
    return el;
  }

  it('measures the reading pane when the main area shows one', () => {
    const main = sized(document.createElement('main'), 1000);
    const pane = sized(document.createElement('div'), 500);
    pane.setAttribute('data-reading-pane', '');
    main.appendChild(pane);
    expect(readingAreaRoom(main)).toBe(500 - READING_AREA_MIN_WIDTH);
  });

  it('measures the main area itself in a view without a reading pane', () => {
    const main = sized(document.createElement('main'), 1000);
    expect(readingAreaRoom(main)).toBe(1000 - READING_AREA_MIN_WIDTH);
  });

  it('gives no room before the main area is mounted', () => {
    expect(readingAreaRoom(null)).toBe(0);
  });
});
