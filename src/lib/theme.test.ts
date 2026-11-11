// The pure side of the appearance theme: parsing the stored preference,
// resolving "system" against the OS, and stamping the result on the document.

import { describe, expect, it } from 'vitest';

import {
  applyScheme,
  DEFAULT_THEME_PREFERENCE,
  parseThemePreference,
  resolveScheme,
  THEME_PREF_KEY,
  THEME_PREFERENCES,
} from './theme';

describe('parseThemePreference', () => {
  it.each(THEME_PREFERENCES)('accepts %s', (value) => {
    expect(parseThemePreference(value)).toBe(value);
  });

  it.each([null, undefined, '', 'auto', 'DARK', 'blue'])('falls back to the default for %s', (raw) => {
    expect(parseThemePreference(raw)).toBe(DEFAULT_THEME_PREFERENCE);
  });

  it('defaults to following the system', () => {
    expect(DEFAULT_THEME_PREFERENCE).toBe('system');
  });

  it('persists under the ui_theme preference key', () => {
    expect(THEME_PREF_KEY).toBe('ui_theme');
  });
});

describe('resolveScheme', () => {
  it.each([
    ['light', false, 'light'],
    ['light', true, 'light'],
    ['dark', false, 'dark'],
    ['dark', true, 'dark'],
    ['system', false, 'light'],
    ['system', true, 'dark'],
  ] as const)('%s with systemDark=%s → %s', (preference, systemDark, expected) => {
    expect(resolveScheme(preference, systemDark)).toBe(expected);
  });
});

describe('applyScheme', () => {
  it('marks the document dark with the class Tailwind variants key on', () => {
    const doc = document.implementation.createHTMLDocument('x');
    applyScheme(doc, 'dark');
    expect(doc.documentElement.classList.contains('dark')).toBe(true);
    expect(doc.documentElement.dataset.theme).toBe('dark');
    expect(doc.documentElement.style.colorScheme).toBe('dark');
  });

  it('removes the dark marker when switching back to light', () => {
    const doc = document.implementation.createHTMLDocument('x');
    applyScheme(doc, 'dark');
    applyScheme(doc, 'light');
    expect(doc.documentElement.classList.contains('dark')).toBe(false);
    expect(doc.documentElement.dataset.theme).toBe('light');
    expect(doc.documentElement.style.colorScheme).toBe('light');
  });

  it('leaves other classes on the root element alone', () => {
    const doc = document.implementation.createHTMLDocument('x');
    doc.documentElement.classList.add('keep-me');
    applyScheme(doc, 'dark');
    applyScheme(doc, 'light');
    expect(doc.documentElement.classList.contains('keep-me')).toBe(true);
  });
});
