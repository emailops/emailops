import { describe, expect, it } from 'vitest';
import { isThemePreference, resolveTheme, type ThemePreference } from './theme';

describe('resolveTheme', () => {
  it('follows the system when asked to', () => {
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
  });

  it('lets an explicit choice override the system', () => {
    // The point of the override: running the app dark on a light desktop (or
    // the reverse) is a normal thing to want.
    expect(resolveTheme('dark', false)).toBe('dark');
    expect(resolveTheme('light', true)).toBe('light');
  });

  it('treats an unrecognised stored value as following the system', () => {
    // The preference is a free-form string in SQLite; a value written by a
    // future version (or a hand-edited row) must not leave the app themeless.
    expect(resolveTheme('sepia' as ThemePreference, true)).toBe('dark');
    expect(resolveTheme('' as ThemePreference, false)).toBe('light');
  });
});

describe('isThemePreference', () => {
  it('accepts the three supported values', () => {
    for (const value of ['light', 'dark', 'system']) {
      expect(isThemePreference(value)).toBe(true);
    }
  });

  it('rejects anything else', () => {
    // Guards the SQLite round-trip: `usePersistedPref` parses with this, so a
    // junk row falls back to the default instead of being applied.
    for (const value of ['', 'Dark', 'sepia', 'true', '1']) {
      expect(isThemePreference(value)).toBe(false);
    }
  });
});
