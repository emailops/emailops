// The appearance theme: light, dark, or follow the system.
//
// The decision is the pure `reduceTheme`; the store around it owns the side
// effects — the `ui_theme` SQLite preference (never localStorage, like every
// other preference), the `dark` class on the document, the native window
// chrome, and the OS appearance listener while the user follows the system.
//
// `init` runs in `main.tsx` before React mounts, so the first frame is already
// in the right theme.

import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { setNativeTheme } from '@/lib/nativeTheme';
import {
  applyScheme,
  DEFAULT_THEME_PREFERENCE,
  parseThemePreference,
  resolveScheme,
  type Scheme,
  type SystemSchemeQuery,
  systemSchemeQuery,
  THEME_PREF_KEY,
  type ThemePreference,
} from '@/lib/theme';
import { useLogStore } from './logStore';

export interface ThemeState {
  preference: ThemePreference;
  scheme: Scheme;
  /** True until the stored preference has been read. */
  isLoading: boolean;
}

export type ThemeAction =
  | { type: 'loaded'; preference: ThemePreference; systemDark: boolean }
  | { type: 'set'; preference: ThemePreference; systemDark: boolean }
  | { type: 'system-changed'; systemDark: boolean };

export const initialThemeState: ThemeState = {
  preference: DEFAULT_THEME_PREFERENCE,
  scheme: 'light',
  isLoading: true,
};

export function reduceTheme(state: ThemeState, action: ThemeAction): ThemeState {
  switch (action.type) {
    case 'loaded':
    case 'set':
      return {
        preference: action.preference,
        scheme: resolveScheme(action.preference, action.systemDark),
        isLoading: false,
      };
    case 'system-changed':
      return { ...state, scheme: resolveScheme(state.preference, action.systemDark) };
  }
}

export const selectScheme = (state: ThemeState): Scheme => state.scheme;
export const selectPreference = (state: ThemeState): ThemePreference => state.preference;

interface ThemeStore extends ThemeState {
  /**
   * Read the stored preference, apply it, and start following the OS
   * appearance. `query` defaults to the window's `prefers-color-scheme`
   * query; pass `null` where there is none.
   */
  init: (query?: SystemSchemeQuery | null) => Promise<void>;
  /** Persist a choice; reverts and throws when it cannot be written. */
  setPreference: (preference: ThemePreference) => Promise<void>;
  /** Stop following the OS appearance (tests; the app never needs it). */
  dispose: () => void;
}

let query: SystemSchemeQuery | null = null;
let onSystemChange: ((e: { matches: boolean }) => void) | null = null;

function systemDark(): boolean {
  return query?.matches ?? false;
}

function applyNative(preference: ThemePreference): void {
  setNativeTheme(preference).catch((err) => {
    useLogStore.getState().addLog('error', 'system', `Could not set the window theme: ${errorText(err)}`);
  });
}

export const useThemeStore = create<ThemeStore>((set, get) => {
  const commit = (next: ThemeState) => {
    applyScheme(document, next.scheme);
    set(next);
  };

  return {
    ...initialThemeState,

    init: async (q = systemSchemeQuery()) => {
      get().dispose();
      query = q;
      onSystemChange = (e) => commit(reduceTheme(get(), { type: 'system-changed', systemDark: e.matches }));
      query?.addEventListener('change', onSystemChange);

      let preference = DEFAULT_THEME_PREFERENCE;
      try {
        preference = parseThemePreference(await api.getPref(THEME_PREF_KEY));
      } catch (err) {
        useLogStore.getState().addLog('error', 'system', `Could not read the theme setting: ${errorText(err)}`);
      }
      commit(reduceTheme(get(), { type: 'loaded', preference, systemDark: systemDark() }));
      applyNative(preference);
    },

    setPreference: async (preference) => {
      const before = get();
      // Optimistic: the screen changes at once; a failed write puts it back so
      // the setting never shows a theme that will not survive a restart.
      commit(reduceTheme(before, { type: 'set', preference, systemDark: systemDark() }));
      try {
        await api.setPref(THEME_PREF_KEY, preference);
      } catch (err) {
        commit({ preference: before.preference, scheme: before.scheme, isLoading: before.isLoading });
        throw err;
      }
      applyNative(preference);
    },

    dispose: () => {
      if (query && onSystemChange) query.removeEventListener('change', onSystemChange);
      query = null;
      onSystemChange = null;
    },
  };
});
