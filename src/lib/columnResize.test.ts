import { describe, expect, it } from 'vitest';
import { clampWidth, dragWidth, KEY_STEP, keyWidth, LIST_COLUMN, parseWidth, SIDEBAR_COLUMN } from './columnResize';

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
});

describe('keyWidth', () => {
  it('steps with the arrows, faster with Shift, and jumps with Home/End', () => {
    expect(keyWidth(384, 'ArrowRight', false, LIST_COLUMN)).toBe(384 + KEY_STEP);
    expect(keyWidth(384, 'ArrowLeft', false, LIST_COLUMN)).toBe(384 - KEY_STEP);
    expect(keyWidth(384, 'ArrowRight', true, LIST_COLUMN)).toBe(384 + KEY_STEP * 10);
    expect(keyWidth(384, 'Home', false, LIST_COLUMN)).toBe(LIST_COLUMN.min);
    expect(keyWidth(384, 'End', false, LIST_COLUMN)).toBe(LIST_COLUMN.max);
  });

  it('ignores other keys', () => {
    expect(keyWidth(384, 'Enter', false, LIST_COLUMN)).toBeNull();
    expect(keyWidth(384, 'ArrowUp', false, LIST_COLUMN)).toBeNull();
  });
});
