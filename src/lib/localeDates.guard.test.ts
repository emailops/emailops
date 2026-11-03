// A date shown to the user follows the UI language. date-fns' `format` without
// a locale always writes en-US ("Nov 14"), whatever the language, and neither
// the JSX-literal guard nor a reader can tell from the call: dates go through
// `useFormatters`.

import { describe, expect, it } from 'vitest';

const SOURCES = import.meta.glob<string>(['/src/**/*.{ts,tsx}', '!/src/**/*.test.*'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

describe('dates on screen', () => {
  it("never use date-fns' en-US format", () => {
    expect(Object.keys(SOURCES).length).toBeGreaterThan(50);
    const offenders = Object.entries(SOURCES)
      .filter(([, text]) => /import \{[^}]*\bformat\b[^}]*\} from 'date-fns'/.test(text))
      .map(([path]) => path);
    expect(offenders).toEqual([]);
  });
});
