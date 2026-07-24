// i18next-parser configuration.
//
// Run `npm run i18n:extract` to walk the codebase and pull every `t('…')`
// key into `src/locales/en/<ns>.json`. Other languages are NOT touched by
// the extractor — the parity Vitest test (`src/i18n/i18n.parity.test.ts`)
// is what guarantees they stay in sync.
//
// We intentionally treat any namespace as valid (no `defaultNamespace`-only
// restriction) because every component declares the namespaces it uses at
// its `useTranslation([...])` call.

/** @type {import('i18next-parser').UserConfig} */
module.exports = {
  // Languages and namespaces — keep in sync with src/i18n/resources.ts.
  locales: ['en', 'es', 'fr', 'de'],
  defaultNamespace: 'common',
  namespaceSeparator: ':',
  keySeparator: '.',

  // Where to write extracted keys. Existing translations are preserved
  // (`resetDefaultValueLocale: undefined`) and only missing keys are added
  // to en. We rely on the parity test for non-en languages.
  output: 'src/locales/$LOCALE/$NAMESPACE.json',
  input: ['src/**/*.{ts,tsx}', '!src/**/*.test.{ts,tsx}', '!src/types/**', '!src/locales/**'],

  // Sort keys for stable diffs.
  sort: true,

  // Stay conservative: never delete keys that the parser doesn't see.
  // Strings can be referenced dynamically (template `${id}` keys for tabs,
  // priority labels, etc.) and we don't want to lose those.
  keepRemoved: true,

  // Don't bother with default-value scaffolding; we maintain the JSON by hand.
  createOldCatalogs: false,
  failOnUpdate: false,
  // Dynamic keys (template `${id}` keys for tabs, priority labels, etc.) always
  // trigger "Key is not a string literal" warnings, so failing on warnings
  // would make every run red. Warnings stay visible as developer nudges only.
  failOnWarnings: false,

  // Lint configuration: warn on dynamic keys we can't statically resolve so
  // the developer notices and adds an `// i18next-extract-mark-key ns:foo`
  // hint where needed.
  lexers: {
    ts: ['JavascriptLexer'],
    tsx: ['JsxLexer'],
    default: ['JavascriptLexer'],
  },
};
