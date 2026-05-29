import { describe, expect, it } from 'vitest';

import { NATIVE_NAMES, SUPPORTED_LANGUAGES } from './resources';

// NATIVE_NAMES is the single frontend source of truth for the language
// selector. These tests guard against drift between it and the supported-
// language list (e.g. adding a language to one but forgetting the other).
describe('NATIVE_NAMES source of truth', () => {
  it('has exactly one entry per supported language', () => {
    expect(Object.keys(NATIVE_NAMES).sort()).toEqual([...SUPPORTED_LANGUAGES].sort());
  });

  it('maps every supported language to a non-empty native name', () => {
    for (const code of SUPPORTED_LANGUAGES) {
      expect(NATIVE_NAMES[code]).toBeTruthy();
    }
  });

  it('has no duplicate native names', () => {
    const names = Object.values(NATIVE_NAMES);
    expect(new Set(names).size).toBe(names.length);
  });
});
