// The app has one fixed look: dark chrome (sidebar, Output bar, Settings and
// every Modal-based dialog) around light content (list, reading pane,
// composers). It does not follow the OS appearance — see docs/DECISIONS.md
// ("Dialogs keep the dark chrome; the app does not follow the OS theme").
//
// Tailwind v4's `dark:` variant is driven by `prefers-color-scheme`, so a
// single `dark:` class would make that element alone flip with the OS setting
// and the surfaces would stop matching each other. Until the app has a real
// theme switch (its own branch, docs/COMPETITOR-PARITY.md), none may appear.

import { describe, expect, it } from 'vitest';

const SOURCES = import.meta.glob<string>(['/src/**/*.tsx', '/src/**/*.ts', '/src/**/*.css', '!/src/**/*.test.*'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

/** A Tailwind `dark:` variant inside a class string (`dark:bg-…`), not the
 *  word "dark" used as a key or in prose. */
const DARK_VARIANT = /(?:^|[\s'"`])dark:[a-z[!-]/m;

describe('no OS-driven dark mode', () => {
  it('reads the sources', () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(100);
  });

  it('uses no Tailwind dark: variant anywhere', () => {
    const offenders = Object.entries(SOURCES)
      .filter(([, source]) => DARK_VARIANT.test(source))
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });

  it('would catch one', () => {
    expect(DARK_VARIANT.test('className="bg-white dark:bg-gray-900"')).toBe(true);
    expect(DARK_VARIANT.test("variant?: 'dark' | 'light';\n  dark: {")).toBe(false);
  });
});
