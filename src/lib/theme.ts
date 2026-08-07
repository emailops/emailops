// Which theme the app renders in.
//
// Pure and DOM-free so the decision is table-testable; `useTheme` owns the
// side effects (reading the OS setting, toggling the class on <html>).

/** What the user chose. Stored in SQLite — see `useTheme`. */
export type ThemePreference = 'light' | 'dark' | 'system';

/** What actually gets rendered. */
export type Theme = 'light' | 'dark';

/** The preference the app defaults to: match the OS until told otherwise. */
export const DEFAULT_THEME_PREFERENCE: ThemePreference = 'system';

/** Whether a stored string is a preference this version understands. */
export function isThemePreference(value: string): value is ThemePreference {
  return value === 'light' || value === 'dark' || value === 'system';
}

/**
 * The theme to render, given the stored preference and what the OS reports.
 *
 * Anything unrecognised follows the system rather than picking a side: the
 * preference is a free-form string in SQLite, and a value written by a future
 * version must not leave the app rendering the wrong theme (or none).
 */
export function resolveTheme(preference: ThemePreference, systemPrefersDark: boolean): Theme {
  if (preference === 'dark') return 'dark';
  if (preference === 'light') return 'light';
  return systemPrefersDark ? 'dark' : 'light';
}
