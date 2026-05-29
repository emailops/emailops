#!/usr/bin/env node
// Strict JSX-literal guard.
//
// Walks every .tsx component under src/ and reports user-visible English text
// that bypasses i18n. Anything flagged here must be moved into a locale
// namespace and called through `t()`.
//
// Three detectors run per file:
//   1. Single-line JSX text:    `>Save<`
//   2. Multi-line JSX text:      text sitting on its own line between tags,
//                                e.g. `</svg>\n  View Email\n</button>`.
//   3. User-visible attributes:  placeholder / title / aria-label / alt / label
//                                with a STRING-LITERAL value (`title="Open"`).
//                                Expression values (`title={x}`) are ignored.
//
// We deliberately scan only `.tsx` files: regex-finding ">…<" inside `.ts`
// catches TypeScript generics (`Promise<void>`) as false positives, and pure
// `.ts` files do not produce JSX anyway.
//
// Allow-list:
//   * Lines containing `// i18n-ignore` are skipped.
//   * Paths under `IGNORED_PATHS` / `IGNORED_FILES` are skipped.
//
// Debug/log strings (`addLog(...)`, `console.*`) are intentionally NOT scanned —
// they surface in the output panel but are out of scope for UI i18n.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const SRC = join(ROOT, 'src');

const IGNORED_PATHS = ['__tests__', '.test.ts', '.test.tsx', '.spec.ts', '.spec.tsx', 'types/generated/', 'locales/', 'i18n/'];

// Single-file escape hatch. Add a path here only with a comment justifying
// why the file is allowed to contain hardcoded English.
const IGNORED_FILES = new Set([
  // (none yet)
]);

// Single-line `>inner<` (no nested JSX delimiters / interpolation).
const JSX_TEXT_RE = />([^<{}>\n]+)</g;

// User-visible attributes whose string-literal values must be translated.
const ATTR_RE = /\b(?:placeholder|title|aria-label|alt|label)\s*=\s*(?:"([^"]*)"|'([^']*)')/g;

// TypeScript-style generics (`Promise<T>`, ...). When inner text starts with
// one of these the "line" is a generic signature, not a JSX text node.
const TS_GENERIC_NAMES = new Set([
  'Promise',
  'Partial',
  'Readonly',
  'ReadonlyArray',
  'ReadonlyMap',
  'ReadonlySet',
  'Record',
  'Map',
  'Set',
  'Array',
  'Pick',
  'Omit',
  'Required',
  'Awaited',
  'Mutable',
  'NonNullable',
  'Exclude',
  'Extract',
  'Parameters',
  'ReturnType',
]);

function isFalsePositive(inner) {
  const trimmed = inner.trim();
  if (!trimmed) return true;
  // No alphabetic character → not text.
  if (!/[A-Za-z]/.test(trimmed)) return true;
  // Starts with digit / operator → JSX expression bleed.
  if (/^[\d=!<>+\-*/%&|^?:;.,]/.test(trimmed)) return true;
  // Operators inside → expression bleed.
  if (/=>|===|==|!==|!=|>=|<=|&&|\|\|/.test(trimmed)) return true;
  if (/;[^A-Za-z]/.test(trimmed) || trimmed.endsWith(';')) return true;
  // TS generic head.
  const firstToken = trimmed.split(/[^A-Za-z]/, 1)[0];
  if (TS_GENERIC_NAMES.has(firstToken)) return true;
  return false;
}

// A "bare prose line" is a JSX text node sitting on its own line: pure words
// and light punctuation, with none of the syntax characters that signal code
// or markup. The JSX-context check (caller) confirms it's between tags.
function isBareProseLine(trimmed) {
  if (!trimmed) return false;
  if (!/[A-Za-z]/.test(trimmed)) return false;
  // Any markup / code syntax disqualifies it (keeps TS generics & expressions out).
  if (/[<>{}=();/"'`|&]/.test(trimmed)) return false;
  if (/^[\d+\-*/%&|^?:.,]/.test(trimmed)) return false;
  // Comment fragments.
  if (trimmed.startsWith('*') || trimmed.startsWith('//')) return false;
  // Code shapes that are not JSX prose:
  //   * a colon → object key, type annotation, or ternary fragment
  //   * trailing comma → import / destructure / array / argument member
  //   * a single bare identifier → import/destructure member (`ChatEvent`)
  if (trimmed.includes(':')) return false;
  if (trimmed.endsWith(',')) return false;
  if (/^[A-Za-z_$][\w$]*$/.test(trimmed)) return false;
  return true;
}

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    if (entry === 'node_modules' || entry.startsWith('.')) continue;
    const full = join(dir, entry);
    const rel = relative(ROOT, full);
    if (IGNORED_PATHS.some((p) => rel.includes(p))) continue;
    const s = statSync(full);
    if (s.isDirectory()) yield* walk(full);
    else if (extname(full) === '.tsx') yield full;
  }
}

const violations = [];

for (const file of walk(SRC)) {
  const rel = relative(ROOT, file);
  if (IGNORED_FILES.has(rel)) continue;
  const lines = readFileSync(file, 'utf8').split('\n');

  // Track block-comment state so prose inside /* ... */ isn't flagged.
  let inBlockComment = false;
  const codeMeta = lines.map((line) => {
    const trimmed = line.trim();
    let isComment = inBlockComment || trimmed.startsWith('//') || trimmed.startsWith('*') || trimmed.startsWith('/*');
    if (inBlockComment && trimmed.includes('*/')) inBlockComment = false;
    else if (!inBlockComment && trimmed.startsWith('/*') && !trimmed.includes('*/')) inBlockComment = true;
    return { trimmed, isComment };
  });

  const prevNonBlank = (i) => {
    for (let j = i - 1; j >= 0; j--) if (codeMeta[j].trimmed) return codeMeta[j];
    return null;
  };
  const nextNonBlank = (i) => {
    for (let j = i + 1; j < lines.length; j++) if (codeMeta[j].trimmed) return codeMeta[j];
    return null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.includes('// i18n-ignore')) continue;
    const { trimmed, isComment } = codeMeta[i];
    if (isComment) continue;
    if (trimmed.startsWith('import ') || trimmed.startsWith('export ')) continue;

    // 1. Single-line JSX text.
    let m;
    JSX_TEXT_RE.lastIndex = 0;
    while ((m = JSX_TEXT_RE.exec(line)) !== null) {
      const inner = m[1].trim();
      if (!isFalsePositive(inner)) violations.push({ file: rel, line: i + 1, text: inner, kind: 'text' });
    }

    // 2. Multi-line JSX text — bare prose line between tags.
    if (isBareProseLine(trimmed)) {
      const prev = prevNonBlank(i);
      const next = nextNonBlank(i);
      const prevOk = prev && !prev.isComment && (prev.trimmed.endsWith('>') || isBareProseLine(prev.trimmed));
      const nextOk = next && (next.trimmed.startsWith('<') || isBareProseLine(next.trimmed));
      if (prevOk && nextOk) violations.push({ file: rel, line: i + 1, text: trimmed, kind: 'text' });
    }

    // 3. User-visible attribute string literals.
    ATTR_RE.lastIndex = 0;
    while ((m = ATTR_RE.exec(line)) !== null) {
      const value = (m[1] ?? m[2] ?? '').trim();
      if (!isFalsePositive(value)) violations.push({ file: rel, line: i + 1, text: value, kind: 'attr' });
    }
  }
}

if (violations.length > 0) {
  console.error(`Found ${violations.length} hardcoded UI literal(s):\n`);
  for (const v of violations) {
    const shown = v.kind === 'attr' ? `(attr) "${v.text}"` : `>${v.text}<`;
    console.error(`  ${v.file}:${v.line}  ${shown}`);
  }
  console.error('\nMove these strings into a locale namespace and call them via t().');
  console.error("If a literal is truly OK (debug UI, code sample, etc.), add `// i18n-ignore` to the line.");
  process.exit(1);
}

console.log('No hardcoded UI literals found.');
