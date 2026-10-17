// Contract test for the dark theme in src/index.css.
//
// The dark theme works by remapping the gray scale under `html.dark` and
// overriding the pale accent utilities (`bg-red-50`, `text-primary-700`…) that
// components put on light surfaces. Neither has a compiler to catch drift, so
// this test does: every gray step must be remapped and restored for pinned
// chrome, and every pale accent class a component uses must have an override.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

const ROOT = join(__dirname, '..');
// Whitespace-collapsed so the assertions read like one-line CSS whatever the
// formatter does to the file.
const css = readFileSync(join(ROOT, 'index.css'), 'utf8').replace(/\s+/g, ' ');

const GRAY_STEPS = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];

/** The `.dark { … }` block and the `.dark :where(.theme-pinned) { … }` block. */
function block(selector: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} block`).toBeGreaterThan(-1);
  return css.slice(start, css.indexOf('}', start));
}

function tsxFiles(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) tsxFiles(p, out);
    else if (p.endsWith('.tsx') && !p.endsWith('.test.tsx')) out.push(p);
  }
  return out;
}

/** The accent classes `index.css` overrides for every colour. */
const OVERRIDDEN =
  /\b(?:hover:)?bg-[a-z]+-(?:50|100)\/[0-9]+\b|\b(?:hover:)?(?:bg-[a-z]+-(?:50|100|200)|text-[a-z]+-(?:600|700|800|900))\b|\bborder-[a-z]+-(?:100|200|300)\b|\b(?:focus:|focus-within:)?ring-[a-z]+-100\b/g;
const NOT_ACCENTS = new Set(['gray', 'neutral', 'slate', 'zinc', 'stone', 'white', 'black']);

describe('dark theme palette (src/index.css)', () => {
  it('keys the dark variant on the html class, not the OS media query', () => {
    expect(css).toContain('@custom-variant dark (&:where(.dark, .dark *));');
  });

  it('defines the brand palette in @theme so utilities compile to variables', () => {
    expect(css).toMatch(/@theme \{[^}]*--color-primary-600: #0284c7;/);
  });

  it.each(GRAY_STEPS)('remaps gray-%s under .dark and restores it under theme-pinned', (step) => {
    expect(block('.dark')).toMatch(new RegExp(`--color-gray-${step}: oklch\\(`));
    expect(block('.dark :where(.theme-pinned)')).toMatch(new RegExp(`--color-gray-${step}: oklch\\(`));
  });

  it('turns bg-white into the dark surface and restores it for pinned chrome', () => {
    expect(css).toContain('.dark :where(.bg-white) { background-color: var(--eo-surface); }');
    expect(block('.dark')).toContain('--eo-surface: oklch(');
    expect(block('.dark :where(.theme-pinned)')).toContain('--eo-surface: #fff');
  });

  it('keeps the gray remap monotonic so layered surfaces stay distinguishable', () => {
    const dark = block('.dark');
    const lightness = (name: string): number => {
      const m = dark.match(new RegExp(`${name}: oklch\\(([0-9.]+)%`));
      if (!m) throw new Error(`${name} missing`);
      return Number(m[1]);
    };
    const scale = ['--color-gray-50', '--eo-surface', ...GRAY_STEPS.slice(1).map((s) => `--color-gray-${s}`)];
    for (let i = 1; i < scale.length; i++) {
      expect(lightness(scale[i]), `${scale[i]} lighter than ${scale[i - 1]}`).toBeGreaterThan(lightness(scale[i - 1]));
    }
  });

  it('overrides every pale accent class the components use', () => {
    const missing = new Set<string>();
    for (const file of tsxFiles(join(ROOT, 'components'))) {
      const src = readFileSync(file, 'utf8');
      for (const cls of src.match(OVERRIDDEN) ?? []) {
        const color = cls.replace(/^(?:hover:|focus:|focus-within:)?(?:bg|text|border|ring)-/, '').split('-')[0];
        if (NOT_ACCENTS.has(color)) continue;
        // The rule for `hover:bg-red-50` is written `.hover\:bg-red-50`.
        // …and `bg-red-50/40` as `.bg-red-50\/40`.
        const escaped = cls.replace(':', '\\:').replace('/', '\\/');
        if (!css.includes(`.${escaped}`)) missing.add(cls);
      }
    }
    expect([...missing].sort()).toEqual([]);
  });
});
