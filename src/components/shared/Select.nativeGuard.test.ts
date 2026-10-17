// Every dropdown goes through the shared Select. A raw <select> renders its
// popup through WebKitGTK's own widget on Linux, which paints it light
// whatever the page's colour scheme — one light popup in a dark app. The
// shared component owns the popup on Linux and keeps the native control on
// macOS and Windows, where the native popup follows `color-scheme`.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

import { describe, expect, it } from 'vitest';

const COMPONENTS = join(__dirname, '..');

function tsxFiles(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) tsxFiles(p, out);
    else if (p.endsWith('.tsx') && !p.endsWith('.test.tsx')) out.push(p);
  }
  return out;
}

describe('native <select> guard', () => {
  it('only Select.tsx renders a raw <select>', () => {
    const offenders = tsxFiles(COMPONENTS)
      .filter((f) => /<select\s/.test(readFileSync(f, 'utf8')))
      .map((f) => relative(COMPONENTS, f))
      .filter((f) => f !== 'shared/Select.tsx');
    expect(offenders).toEqual([]);
  });
});
