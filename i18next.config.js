// i18next-cli configuration.
//
// Run `npm run i18n:extract` to walk the codebase and pull every `t('…')`
// key into `src/locales/<lang>/<ns>.json`. Translations already in the
// catalogs are kept; a key new to the code lands in en and, empty, in the
// other languages — the parity Vitest test (`src/i18n/i18n.parity.test.ts`)
// is what guarantees they are filled in.
//
// Any namespace is valid: every component declares the namespaces it uses at
// its `useTranslation([...])` call.

import { defineConfig } from 'i18next-cli';

export default defineConfig({
  // Languages — keep in sync with src/i18n/resources.ts.
  locales: ['en', 'es', 'fr', 'de'],
  extract: {
    input: ['src/**/*.{ts,tsx}'],
    ignore: ['src/**/*.test.{ts,tsx}', 'src/types/**', 'src/locales/**'],
    output: 'src/locales/{{language}}/{{namespace}}.json',
    defaultNS: 'common',
    nsSeparator: ':',
    keySeparator: '.',
    // Sort keys for stable diffs.
    sort: true,
    // Never delete keys the extractor doesn't see: strings are also referenced
    // dynamically (template `${id}` keys for tabs, priority labels, etc.).
    removeUnusedKeys: false,
    // `t(key, { count })` also scaffolds `key_one`/`key_other`… holding the key
    // name. The catalogs keep the bare `key` (i18next falls back to it), so do
    // not commit those scaffolds; `npm run i18n:check` ignores them.
  },
});
