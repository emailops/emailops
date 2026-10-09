#!/usr/bin/env node
// i18n drift check (`npm run i18n:check`).
//
// Runs the same extraction as `npm run i18n:extract` (i18next-cli, with the
// project's i18next.config.js) against a temp copy of the locale catalogs,
// then compares semantically. `src/locales/` is never touched.
//
// The comparison is key-based, not byte-based, because the extractor's
// canonical output can never match the catalogs exactly:
//   * `sort: true` reorders keys, while the checked-in files are hand-ordered.
//   * The extractor scaffolds plural-suffix keys (`key_one`, `key_many`,
//     `key_other`) for every `t(key, { count })` call, with the key itself as
//     the value. The catalogs intentionally keep the bare `key` instead —
//     i18next falls back to it at runtime — and committing the scaffolds would
//     replace real text with the key name.
//
// So we fail only on real drift: a key the code uses that no catalog defines
// (typo or forgotten addition), or a value extraction would change.

import { cpSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runExtractor } from 'i18next-cli';
import config from '../i18next.config.js';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const LOCALES = join(ROOT, 'src', 'locales');

const PLURAL_SUFFIX_RE = /_(zero|one|two|few|many|other)$/;

function flatten(obj, prefix = '', out = {}) {
  for (const [key, value] of Object.entries(obj)) {
    if (value !== null && typeof value === 'object') flatten(value, `${prefix}${key}.`, out);
    else out[`${prefix}${key}`] = value;
  }
  return out;
}

function readCatalog(dir, locale, file) {
  try {
    return flatten(JSON.parse(readFileSync(join(dir, locale, file), 'utf8')));
  } catch {
    return null; // namespace file missing entirely
  }
}

const tmp = mkdtempSync(join(tmpdir(), 'i18n-check-'));
const violations = [];
try {
  // Seed the temp output with the real catalogs so the extractor merges
  // existing translations exactly as `i18n:extract` would.
  cpSync(LOCALES, join(tmp, 'locales'), { recursive: true });
  const output = join(tmp, 'locales', '{{language}}', '{{namespace}}.json');
  // The temp files it rewrites are not news; warnings and errors still show.
  const logger = { info: () => {}, warn: console.warn, error: console.error };
  const { hasErrors } = await runExtractor({ ...config, extract: { ...config.extract, output } }, { quiet: true, logger });
  if (hasErrors) violations.push('the extractor reported errors (see above)');

  for (const locale of readdirSync(join(tmp, 'locales'))) {
    for (const file of readdirSync(join(tmp, 'locales', locale))) {
      const extracted = flatten(JSON.parse(readFileSync(join(tmp, 'locales', locale, file), 'utf8')));
      const catalog = readCatalog(LOCALES, locale, file);
      if (catalog === null) {
        violations.push(`${locale}/${file}: namespace used in code but has no catalog file`);
        continue;
      }
      for (const [key, value] of Object.entries(extracted)) {
        if (key in catalog) {
          if (catalog[key] !== value) {
            violations.push(`${locale}/${file}: "${key}" would change: ${JSON.stringify(catalog[key])} -> ${JSON.stringify(value)}`);
          }
          continue;
        }
        // Extractor-scaffolded plural variant the catalog covers: by its bare
        // key, or by another plural form of it (the extractor emits every CLDR
        // form, e.g. `_many` for fr/es, where the catalogs hold `_one`/`_other`).
        if (PLURAL_SUFFIX_RE.test(key)) {
          const base = key.replace(PLURAL_SUFFIX_RE, '');
          if (base in catalog || Object.keys(catalog).some((k) => k.replace(PLURAL_SUFFIX_RE, '') === base && k !== base)) continue;
        }
        violations.push(`${locale}/${file}: missing key "${key}"`);
      }
    }
  }
} finally {
  rmSync(tmp, { recursive: true, force: true });
}

if (violations.length > 0) {
  console.error(`i18n:check failed — ${violations.length} drift issue(s):\n`);
  for (const v of violations) console.error(`  ${v}`);
  console.error('\nAdd the missing keys to src/locales/ (all four languages).');
  process.exit(1);
}
console.log('i18n:check OK — every key used in code is covered by the locale catalogs.');
