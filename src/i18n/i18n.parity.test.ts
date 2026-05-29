// Key parity test for every locale namespace.
//
// English is the source of truth (it's also what `src/types/i18next.d.ts`
// types `t()` against). Every other supported language must have exactly the
// same set of keys per namespace — same nesting, same placeholders. If a
// translator forgets a key the build fails, which is the whole point.

import { describe, expect, it } from 'vitest';

import { FALLBACK_LANGUAGE, NAMESPACES, resources, SUPPORTED_LANGUAGES } from './resources';

/** Walk a translations tree and return a sorted list of `a.b.c` leaf paths. */
function leafKeys(node: unknown, prefix = ''): string[] {
  if (node === null || typeof node !== 'object') {
    return [prefix];
  }
  const out: string[] = [];
  for (const [k, v] of Object.entries(node as Record<string, unknown>)) {
    const path = prefix ? `${prefix}.${k}` : k;
    out.push(...leafKeys(v, path));
  }
  return out.sort();
}

/** Pull every `{{name}}` placeholder out of a translation string. */
function placeholders(value: unknown): string[] {
  if (typeof value !== 'string') return [];
  const out = new Set<string>();
  for (const match of value.matchAll(/\{\{\s*([\w.-]+)\s*\}\}/g)) {
    out.add(match[1]);
  }
  return [...out].sort();
}

/** Fetch the value at `a.b.c` from a nested translations object. */
function lookup(tree: unknown, path: string): unknown {
  return path.split('.').reduce<unknown>((acc, segment) => {
    if (acc && typeof acc === 'object' && segment in (acc as Record<string, unknown>)) {
      return (acc as Record<string, unknown>)[segment];
    }
    return undefined;
  }, tree);
}

const NON_DEFAULT_LANGUAGES = SUPPORTED_LANGUAGES.filter((l) => l !== FALLBACK_LANGUAGE);

describe('i18n locale parity', () => {
  for (const ns of NAMESPACES) {
    const enKeys = leafKeys(resources[FALLBACK_LANGUAGE][ns]);

    describe(`namespace "${ns}"`, () => {
      it.each(NON_DEFAULT_LANGUAGES)('%s has the same key set as %s', (lang) => {
        const langKeys = leafKeys(resources[lang][ns]);
        expect(langKeys).toEqual(enKeys);
      });

      it.each(NON_DEFAULT_LANGUAGES)('%s preserves placeholders from %s', (lang) => {
        for (const key of enKeys) {
          const enPlaceholders = placeholders(lookup(resources[FALLBACK_LANGUAGE][ns], key));
          const langPlaceholders = placeholders(lookup(resources[lang][ns], key));
          expect(langPlaceholders, `placeholders for ${ns}:${key} in ${lang}`).toEqual(enPlaceholders);
        }
      });
    });
  }

  it('every namespace is registered for every language', () => {
    for (const lang of SUPPORTED_LANGUAGES) {
      const registered = Object.keys(resources[lang]).sort();
      expect(registered, `namespaces registered for ${lang}`).toEqual([...NAMESPACES].sort());
    }
  });
});
