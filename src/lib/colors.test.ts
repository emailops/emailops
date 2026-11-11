// Unit tests for the deterministic color-hash helpers.

import { describe, expect, it } from 'vitest';

import {
  ACCOUNT_COLORS,
  AVATAR_PALETTE,
  accountColorClass,
  accountColorHex,
  hashColorClass,
  SENDER_TEXT_PALETTE,
  senderTextColorClass,
} from './colors';

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
  it("paints an account with its stored colour's class", () => {
    const accounts = [{ id: 'acc-1', color: 'rose' as const }];
    expect(accountColorClass(accounts, 'acc-1')).toBe('bg-rose-500');
  });

  it('falls back to a neutral class for an account it does not know', () => {
    expect(accountColorClass([], 'gone')).toBe('bg-gray-400');
  });
});

describe('accountColorHex', () => {
  it('gives every palette colour a hex for the calendar chips', () => {
    for (const color of ACCOUNT_COLORS) {
      expect(accountColorHex(color)).toMatch(/^#[0-9a-f]{6}$/);
    }
  });
});

describe('senderTextColorClass', () => {
  it('is deterministic for the same sender', () => {
    expect(senderTextColorClass('alice@example.com')).toBe(senderTextColorClass('alice@example.com'));
  });

  it('returns a Tailwind text- class, not a background', () => {
    // These colour the sender NAME itself, so a bg- class would paint a block.
    expect(senderTextColorClass('alice@example.com')).toMatch(/^text-/);
  });

  it('gives different senders different colours', () => {
    const seeds = Array.from({ length: 30 }, (_, i) => `person-${i}@example.com`);
    const distinct = new Set(seeds.map(senderTextColorClass));
    expect(distinct.size).toBeGreaterThan(3);
  });

  it('is case-insensitive — one sender keeps one colour across header casings', () => {
    // Providers vary the case of the From address; a colour that flips with it
    // would make the same person look like two people down a column.
    expect(senderTextColorClass('Alice@Example.com')).toBe(senderTextColorClass('alice@example.com'));
  });

  it('falls back to a palette colour for an empty seed', () => {
    expect(SENDER_TEXT_PALETTE).toContain(senderTextColorClass(''));
  });
});
