// The appearance theme, pure side: what the user can choose, how "system"
// resolves against the OS, and how the result is stamped on the document.
//
// The whole dark theme hangs on one class: `html.dark`. `src/index.css` remaps
// the gray palette and the light accent tints under it, so the hundreds of
// `bg-white` / `text-gray-*` utilities in the components turn dark without a
// `dark:` twin each (see the "Dark theme" section there). Every screen is
// written in that light idiom, so all of them share the inbox's surfaces; the
// sidebar and the dialogs (`theme-chrome`) get the dark palette on either theme.

export const THEME_PREFERENCES = ['light', 'dark', 'system'] as const;
export type ThemePreference = (typeof THEME_PREFERENCES)[number];

/** What is actually on screen once "system" has been resolved. */
export type Scheme = 'light' | 'dark';

/** SQLite preference key (`user_preferences`), read before React mounts. */
export const THEME_PREF_KEY = 'ui_theme';

export const DEFAULT_THEME_PREFERENCE: ThemePreference = 'system';

export function parseThemePreference(raw: string | null | undefined): ThemePreference {
  return (THEME_PREFERENCES as readonly string[]).includes(raw ?? '')
    ? (raw as ThemePreference)
    : DEFAULT_THEME_PREFERENCE;
}

export function resolveScheme(preference: ThemePreference, systemDark: boolean): Scheme {
  if (preference === 'system') return systemDark ? 'dark' : 'light';
  return preference;
}

/**
 * Stamp the scheme on the root element: the `dark` class Tailwind's `dark:`
 * variant and the palette remap key on, `data-theme` for anything that wants
 * to read it, and `color-scheme` so native controls and scrollbars follow.
 */
export function applyScheme(doc: Document, scheme: Scheme): void {
  const root = doc.documentElement;
  root.classList.toggle('dark', scheme === 'dark');
  root.dataset.theme = scheme;
  root.style.colorScheme = scheme;
}

/** The subset of `MediaQueryList` the store needs, so tests can fake it. */
export interface SystemSchemeQuery {
  readonly matches: boolean;
  addEventListener(type: 'change', listener: (e: { matches: boolean }) => void): void;
  removeEventListener(type: 'change', listener: (e: { matches: boolean }) => void): void;
}

/** `null` where `matchMedia` is missing (jsdom, very old webviews): treated as light. */
export function systemSchemeQuery(win: Window = window): SystemSchemeQuery | null {
  return typeof win.matchMedia === 'function' ? win.matchMedia('(prefers-color-scheme: dark)') : null;
}
