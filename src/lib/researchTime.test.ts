import { describe, expect, it } from 'vitest';
import { formatDuration, remainingSeconds } from './researchTime';

describe('formatDuration', () => {
  it('rounds to what a person reads at a glance', () => {
    expect(formatDuration(20)).toBe('<1 min');
    expect(formatDuration(90)).toBe('2 min');
    expect(formatDuration(35 * 60)).toBe('35 min');
    expect(formatDuration(2 * 3600 + 10 * 60)).toBe('2 h 10 min');
    expect(formatDuration(3 * 3600)).toBe('3 h');
  });
});

describe('remainingSeconds', () => {
  it('extrapolates from the pace so far', () => {
    // 20 of 100 read in 40 s → 80 left at 2 s each.
    expect(remainingSeconds({ emailsRead: 20, emailsTotal: 100 }, 0, 40_000)).toBe(160);
  });

  it('has no estimate before anything was read', () => {
    expect(remainingSeconds({ emailsRead: 0, emailsTotal: 100 }, 0, 40_000)).toBeNull();
  });
});
