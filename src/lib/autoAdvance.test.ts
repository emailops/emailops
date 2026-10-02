import { describe, expect, it } from 'vitest';
import { type AfterLeaveMode, parseAfterLeaveMode, planAdvance } from './autoAdvance';

const LIST = ['a', 'b', 'c', 'd'];
const none = new Set<string>();

describe('planAdvance', () => {
  it.each([
    // [list, leaving, gone, mode, expected]
    [LIST, 'b', none, 'next', 'c'],
    [LIST, 'a', none, 'next', 'b'],
    [LIST, 'd', none, 'next', 'c'], // end of the list: the previous one
    [LIST, 'b', none, 'previous', 'a'],
    [LIST, 'a', none, 'previous', 'b'], // top of the list: the next one
    [LIST, 'b', none, 'list', null],
    [['a'], 'a', none, 'next', null], // the list is now empty
    [[], 'a', none, 'next', null],
    [LIST, 'z', none, 'next', null], // opened from outside the list
    [LIST, 'b', new Set(['c']), 'next', 'd'], // rows that left with it are skipped
    [LIST, 'b', new Set(['c', 'd']), 'next', 'a'],
    [LIST, 'c', new Set(['a', 'b', 'd']), 'previous', null],
  ] as [string[], string, Set<string>, AfterLeaveMode, string | null][])(
    '%j leaving %s (gone %j, %s) → %s',
    (list, leaving, gone, mode, expected) => {
      expect(planAdvance(list, leaving, gone, mode)).toBe(expected);
    },
  );
});

describe('parseAfterLeaveMode', () => {
  it.each([
    [null, 'next'],
    ['', 'next'],
    ['bogus', 'next'],
    ['next', 'next'],
    ['previous', 'previous'],
    ['list', 'list'],
  ])('%s → %s', (raw, mode) => {
    expect(parseAfterLeaveMode(raw)).toBe(mode);
  });
});
