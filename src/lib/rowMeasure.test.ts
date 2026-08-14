import { describe, expect, it } from 'vitest';
import { measuredRowHeight } from './rowMeasure';

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
