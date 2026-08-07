// Every light *surface* in a themed component must have a dark counterpart.
//
// This exists because the failure is invisible to every other check. A missing
// `dark:` variant type-checks, lints, renders, and passes every unit test — it
// only shows up as a white slab on screen, and only if someone happens to open
// that view in dark mode. Three separate audits during the original migration
// reported "complete" while the inbox was visibly broken.
//
// Scope is deliberately surfaces (backgrounds and gradient stops), not text:
// a missing background is what produces an unreadable white block, while a
// grey that is one step off is a cosmetic nit. Only files that already contain
// `dark:bg-` are checked, so the app's intentionally-dark chrome (sidebar,
// settings, dialogs) is not dragged in.

import { describe, expect, it } from 'vitest';

/** Every component's source, read at build time. Vite's glob rather than
 *  `node:fs`: the app's tsconfig carries no Node types, and this keeps the
 *  scan working the same way under vitest and any future browser runner. */
const SOURCES = import.meta.glob('../components/**/*.tsx', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

/** Light surface utilities, with the dark class each one requires. */
const REQUIRED: Record<string, string> = {
  'bg-white': 'bg-surface',
  'bg-gray-50': 'bg-surface-raised',
  'bg-gray-100': 'bg-surface-hover',
  'from-white': 'from-surface',
  'to-white': 'to-surface',
  'from-gray-50': 'from-surface-raised',
  'to-gray-50': 'to-surface-raised',
};

const TOKEN = new RegExp(
  `(?<![\\w:/-])((?:[a-z][a-z0-9-]*:)*)(${Object.keys(REQUIRED).join('|')})(/\\d{1,3})?(?![\\w/-])`,
  'g',
);

describe('dark mode surface coverage', () => {
  it('gives every light surface in a themed file a dark counterpart', () => {
    const gaps: string[] = [];

    for (const [file, source] of Object.entries(SOURCES)) {
      if (file.includes('.test.')) continue;
      // A file with no dark backgrounds at all is chrome that was authored
      // dark, or has no surfaces; either way it is not part of this migration.
      if (!source.includes('dark:bg-')) continue;

      const lines = source.split('\n');
      lines.forEach((line, index) => {
        for (const match of line.matchAll(TOKEN)) {
          const [, prefix, token, opacity] = match;
          if (prefix.includes('dark:')) continue;
          let mapped = REQUIRED[token];
          if (opacity) mapped += opacity;
          const want = `dark:${prefix}${mapped}`;
          // The counterpart normally sits in the same class string; a window
          // covers class lists that biome has wrapped across lines.
          const window = lines.slice(Math.max(0, index - 4), index + 5).join('\n');
          if (!window.includes(want)) {
            gaps.push(`${file.replace('../', 'src/')}:${index + 1} — "${match[0]}" needs "${want}"`);
          }
        }
      });
    }

    expect(gaps, `light surfaces with no dark counterpart:\n${gaps.join('\n')}`).toEqual([]);
  });
});
