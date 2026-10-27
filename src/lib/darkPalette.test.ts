// Contract test for the dark theme in src/index.css.
//
// The dark theme works by remapping the gray scale under `html.dark` and
// overriding the pale accent utilities (`bg-red-50`, `text-primary-700`…) that
// components put on light surfaces. The sidebar and the dialogs (`theme-chrome`)
// get the same remap on the light theme too, so they stay dark there. Neither
// has a compiler to catch drift, so this test does: every gray step must be
// remapped, and every pale accent class a component uses must have an override.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

import { describe, expect, it } from 'vitest';

const ROOT = join(__dirname, '..');
// Whitespace-collapsed so the assertions read like one-line CSS whatever the
// formatter does to the file.
const css = readFileSync(join(ROOT, 'index.css'), 'utf8').replace(/\s+/g, ' ');

/** The variable block: the dark theme, and the chrome on either theme. */
const PALETTE = '.dark, .theme-chrome';

const GRAY_STEPS = ['50', '100', '200', '300', '400', '500', '600', '700', '800', '900', '950'];

/** The rule block for `selector`, up to its first closing brace. */
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
  it('keys the dark variant on the html class and the chrome, not the OS media query', () => {
    expect(css).toContain('@custom-variant dark (&:where(.dark, .dark *, .theme-chrome, .theme-chrome *));');
  });

  it('defines the brand palette in @theme so utilities compile to variables', () => {
    expect(css).toMatch(/@theme \{[^}]*--color-primary-600: #0284c7;/);
  });

  it.each(GRAY_STEPS)('remaps gray-%s under .dark and inside the chrome', (step) => {
    expect(block(PALETTE)).toMatch(new RegExp(`--color-gray-${step}: oklch\\(`));
  });

  it('turns bg-white into the dark surface, on the chrome root as well', () => {
    expect(css).toContain(':is(.dark, .theme-chrome) :where(.bg-white) { background-color: var(--eo-surface); }');
    expect(css).toContain('.theme-chrome:where(.bg-white) { background-color: var(--eo-surface); }');
    expect(block(PALETTE)).toContain('--eo-surface: oklch(');
  });

  it('gives every dark override to the chrome too', () => {
    expect(css).not.toMatch(/(?<![(,] ?)\.dark :where\(/);
  });

  it('keeps the gray remap monotonic so layered surfaces stay distinguishable', () => {
    const dark = block(PALETTE);
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

  // Every screen shares the inbox's surfaces (white on light, navy on dark), so
  // a colour that cannot follow the theme — a pinned subtree, a hex literal, a
  // scale the remap leaves alone — would make one screen differ from the rest.
  it('keeps every screen on the theme-following palette', () => {
    const offenders: string[] = [];
    const FIXED = /\btheme-pinned\b|\b[a-z-]+-\[#[0-9a-fA-F]{3,8}\]|\b[a-z-]+-(?:neutral|zinc|slate|stone)-[0-9]+\b/g;
    for (const file of tsxFiles(join(ROOT, 'components'))) {
      for (const cls of readFileSync(file, 'utf8').match(FIXED) ?? []) {
        offenders.push(`${file.slice(ROOT.length + 1)}: ${cls}`);
      }
    }
    expect([...new Set(offenders)].sort()).toEqual([]);
  });

  // The sidebar, the log panel and the dialogs keep their dark look on the light theme.
  it.each([
    'Sidebar/Sidebar.tsx',
    'LogPanel/LogPanel.tsx',
    'common/Modal.tsx',
    'Settings/SettingsDialog.tsx',
    'Sidebar/AccountSettingsDialog.tsx',
    'Sidebar/AddAccountModal.tsx',
    'AddImapAccountModal.tsx',
    'shared/AiWorkInProgressDialog.tsx',
  ])('%s carries theme-chrome', (file) => {
    expect(readFileSync(join(ROOT, 'components', file), 'utf8')).toMatch(/\btheme-chrome\b/);
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

  // `!bg-primary-50` compiles to its own `!important` rule, which the dark
  // override for `.bg-primary-50` neither matches nor outranks: the light tint
  // stays behind the dark theme's light text.
  it('never forces a pale accent class with the important modifier', () => {
    const pale = new RegExp(`^(?:${OVERRIDDEN.source})$`);
    const IMPORTANT =
      /(?<![\w-])!((?:bg|text|border|ring)-[a-z]+-[0-9]+(?:\/[0-9]+)?)|\b((?:bg|text|border|ring)-[a-z]+-[0-9]+(?:\/[0-9]+)?)!/g;
    const forced: string[] = [];
    for (const file of tsxFiles(join(ROOT, 'components'))) {
      for (const m of readFileSync(file, 'utf8').matchAll(IMPORTANT)) {
        const cls = m[1] ?? m[2];
        if (pale.test(cls) && !NOT_ACCENTS.has(cls.split('-')[1])) {
          forced.push(`${file.slice(ROOT.length + 1)}: ${m[0]}`);
        }
      }
    }
    expect([...new Set(forced)].sort()).toEqual([]);
  });
});
