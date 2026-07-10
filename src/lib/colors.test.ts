// Unit tests for the deterministic color-hash helpers.

import { describe, expect, it } from 'vitest';

import { AVATAR_PALETTE, accountColorClass, hashColorClass } from './colors';

describe('hashColorClass', () => {
  it('is deterministic for the same seed', () => {
    expect(hashColorClass('alice@example.com', AVATAR_PALETTE)).toBe(
      hashColorClass('alice@example.com', AVATAR_PALETTE),
    );
  });

  it('always returns a class from the given palette', () => {
    const palette = ['a', 'b', 'c'];
    for (const seed of ['', 'x', 'hello world', 'ünïcødé@exämple.com']) {
      expect(palette).toContain(hashColorClass(seed, palette));
    }
  });

  it('spreads distinct seeds across the palette', () => {
    const seeds = Array.from({ length: 40 }, (_, i) => `account-${i}@example.com`);
    const distinct = new Set(seeds.map((s) => hashColorClass(s, AVATAR_PALETTE)));
    expect(distinct.size).toBeGreaterThan(1);
  });
});

describe('accountColorClass', () => {
  it('is deterministic and stable across calls', () => {
    expect(accountColorClass('acc-1')).toBe(accountColorClass('acc-1'));
  });

  it('returns a Tailwind bg- class', () => {
    expect(accountColorClass('acc-1')).toMatch(/^bg-/);
  });
});
