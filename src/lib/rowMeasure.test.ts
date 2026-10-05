import { describe, expect, it } from 'vitest';
import { measuredRowHeight, visibleScrollRect } from './rowMeasure';

describe('measuredRowHeight', () => {
  it('takes the measurement when the row has a real box', () => {
    expect(measuredRowHeight({ measured: 45, cached: 48, estimate: 48 })).toBe(45);
  });

  it('keeps the last known height when the row measures zero', () => {
    expect(measuredRowHeight({ measured: 0, cached: 45, estimate: 48 })).toBe(45);
  });

  it('falls back to the estimate when there is nothing cached yet', () => {
    expect(measuredRowHeight({ measured: 0, cached: undefined, estimate: 48 })).toBe(48);
  });

  it('falls back to the estimate when the cache itself was already poisoned', () => {
    expect(measuredRowHeight({ measured: 0, cached: 0, estimate: 48 })).toBe(48);
  });
});

// The list's own box goes 0x0 while App hides it (full-width layout with a
// conversation or compose tab open). The virtualizer turns a zero viewport
// into "no rows", so the list came back blank above "No more emails" until a
// ResizeObserver callback caught up — seconds, in a busy or occluded window.
describe('visibleScrollRect', () => {
  const shown = { width: 900, height: 640 };

  it('passes a real viewport through', () => {
    expect(visibleScrollRect({ width: 900, height: 500 }, shown)).toEqual({ width: 900, height: 500 });
  });

  it('keeps the last real viewport while the list is hidden', () => {
    expect(visibleScrollRect({ width: 0, height: 0 }, shown)).toEqual(shown);
  });

  it('reports the zero box when the list was never shown', () => {
    expect(visibleScrollRect({ width: 0, height: 0 }, null)).toEqual({ width: 0, height: 0 });
  });
});
