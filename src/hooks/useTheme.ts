// Applies the resolved theme to the document, and keeps it in step with the OS.
//
// The decision itself is `lib/theme.ts`; this owns only the side effects. Call
// it once, at the app root — a second caller would fight the first over the
// class on <html>.

import { useEffect, useState } from 'react';
import {
  DEFAULT_THEME_PREFERENCE,
  isThemePreference,
  resolveTheme,
  type Theme,
  type ThemePreference,
} from '@/lib/theme';
import { useThemeStore } from '@/stores/themeStore';
import { usePersistedPref } from './usePersistedPref';

const MEDIA_QUERY = '(prefers-color-scheme: dark)';

/** `matchMedia` is absent in jsdom unless a test stubs it; treating that as
 *  "light" keeps component tests rendering a theme rather than throwing. */
function systemPrefersDark(): boolean {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(MEDIA_QUERY).matches
    : false;
}

export interface UseTheme {
  /** What is actually rendered right now. */
  theme: Theme;
  /** What the user chose — `system` until they choose otherwise. */
  preference: ThemePreference;
  setPreference: (next: ThemePreference) => void;
  /** False until the stored preference has been read back from SQLite. */
  isLoaded: boolean;
}

/**
 * The active theme, the stored preference, and a setter.
 *
 * Tailwind runs with `darkMode: 'class'`, so everything downstream is a `dark:`
 * variant keyed off one class on the root element.
 */
export function useTheme(): UseTheme {
  const [preference, setPreference, isLoaded] = usePersistedPref<ThemePreference>(
    'appearance_theme',
    DEFAULT_THEME_PREFERENCE,
    {
      parse: (raw) => (isThemePreference(raw) ? raw : null),
      serialize: (value) => value,
    },
  );

  const [systemDark, setSystemDark] = useState<boolean>(systemPrefersDark);

  // Track the OS setting even when not following it: the user can switch the
  // preference back to `system` at any time, and the answer must already be
  // current rather than stale until the next OS change.
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const query = window.matchMedia(MEDIA_QUERY);
    setSystemDark(query.matches);
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  const theme = resolveTheme(preference, systemDark);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle('dark', theme === 'dark');
    // The matching `light` class is not what Tailwind keys off — it exists so
    // the root background rule in index.css can outrank the
    // `prefers-color-scheme` fallback that paints the pre-mount frames. Without
    // it, choosing Light on a system set to dark leaves a dark root showing
    // through the overscroll gutter.
    root.classList.toggle('light', theme === 'light');
    // Tells the UA to render its own furniture — scrollbars, form controls,
    // the caret — in the matching scheme. Without it a dark app keeps white
    // scrollbars.
    root.style.colorScheme = theme;
    // Publish for readers that cannot use a `dark:` class — notably the email
    // frame, whose document Tailwind never sees.
    useThemeStore.getState().setTheme(theme);
  }, [theme]);

  return { theme, preference, setPreference, isLoaded };
}
