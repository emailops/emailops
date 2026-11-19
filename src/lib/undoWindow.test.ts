import { describe, expect, it } from 'vitest';
import { DEFAULT_UNDO_WINDOW_SECS, parseUndoWindow, UNDO_WINDOW_OPTIONS } from './undoWindow';

describe('parseUndoWindow', () => {
  it('defaults to 6 s when the preference is unset or not an offered window', () => {
    expect(DEFAULT_UNDO_WINDOW_SECS).toBe(6);
    for (const raw of [null, undefined, '', ' ', 'abc', '5', '-6', '60', '6.5']) {
      expect(parseUndoWindow(raw)).toBe(6);
    }
  });

  it('reads off and every offered window', () => {
    expect(UNDO_WINDOW_OPTIONS).toEqual([0, 6, 10, 20, 30]);
    for (const secs of UNDO_WINDOW_OPTIONS) expect(parseUndoWindow(String(secs))).toBe(secs);
  });
});
