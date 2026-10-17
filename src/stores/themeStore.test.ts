// The theme store: the pure reducer that turns preference + OS appearance into
// the scheme on screen, and the thin actions around it (SQLite preference,
// document class, native window chrome, OS appearance changes).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));

const setNativeTheme = vi.fn(async (_pref: string) => {});
vi.mock('@/lib/nativeTheme', () => ({
  setNativeTheme: (pref: string) => setNativeTheme(pref),
}));

import * as api from '@/lib/api';
import { useLogStore } from './logStore';
import { initialThemeState, reduceTheme, selectScheme, type ThemeState, useThemeStore } from './themeStore';

/** A controllable stand-in for `matchMedia('(prefers-color-scheme: dark)')`. */
function fakeSystemScheme(dark: boolean) {
  const listeners = new Set<(e: { matches: boolean }) => void>();
  const mql = {
    matches: dark,
    addEventListener: (_: 'change', cb: (e: { matches: boolean }) => void) => listeners.add(cb),
    removeEventListener: (_: 'change', cb: (e: { matches: boolean }) => void) => listeners.delete(cb),
  };
  return {
    mql,
    flip(next: boolean) {
      mql.matches = next;
      for (const cb of listeners) cb({ matches: next });
    },
  };
}

describe('reduceTheme (pure)', () => {
  const base: ThemeState = { ...initialThemeState };

  it('starts following the system, light, and loading', () => {
    expect(base).toEqual({ preference: 'system', scheme: 'light', isLoading: true });
  });

  it('loaded: resolves a stored preference against the OS appearance', () => {
    expect(reduceTheme(base, { type: 'loaded', preference: 'dark', systemDark: false })).toEqual({
      preference: 'dark',
      scheme: 'dark',
      isLoading: false,
    });
    expect(reduceTheme(base, { type: 'loaded', preference: 'system', systemDark: true })).toEqual({
      preference: 'system',
      scheme: 'dark',
      isLoading: false,
    });
  });

  it('set: an explicit choice ignores the OS appearance', () => {
    const dark = reduceTheme(base, { type: 'set', preference: 'light', systemDark: true });
    expect(selectScheme(dark)).toBe('light');
  });

  it('system-changed: only moves the scheme while following the system', () => {
    const following = reduceTheme(base, { type: 'loaded', preference: 'system', systemDark: false });
    expect(selectScheme(reduceTheme(following, { type: 'system-changed', systemDark: true }))).toBe('dark');

    const pinned = reduceTheme(base, { type: 'loaded', preference: 'light', systemDark: false });
    expect(selectScheme(reduceTheme(pinned, { type: 'system-changed', systemDark: true }))).toBe('light');
  });
});

describe('useThemeStore', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useThemeStore.setState({ ...initialThemeState });
    document.documentElement.classList.remove('dark');
    delete document.documentElement.dataset.theme;
    useLogStore.setState({ entries: [] });
  });
  afterEach(() => {
    useThemeStore.getState().dispose();
  });

  it('init reads ui_theme, applies it to the document and the window chrome', async () => {
    vi.mocked(api.getPref).mockResolvedValueOnce('dark');
    const system = fakeSystemScheme(false);

    await useThemeStore.getState().init(system.mql);

    expect(api.getPref).toHaveBeenCalledWith('ui_theme');
    expect(useThemeStore.getState()).toMatchObject({ preference: 'dark', scheme: 'dark', isLoading: false });
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(setNativeTheme).toHaveBeenCalledWith('dark');
  });

  it('init with no stored preference follows the OS appearance', async () => {
    const system = fakeSystemScheme(true);
    await useThemeStore.getState().init(system.mql);
    expect(useThemeStore.getState()).toMatchObject({ preference: 'system', scheme: 'dark' });
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('follows OS appearance changes while on system, and stops once a choice is pinned', async () => {
    const system = fakeSystemScheme(false);
    await useThemeStore.getState().init(system.mql);
    expect(document.documentElement.classList.contains('dark')).toBe(false);

    system.flip(true);
    expect(useThemeStore.getState().scheme).toBe('dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);

    await useThemeStore.getState().setPreference('light');
    system.flip(false);
    system.flip(true);
    expect(useThemeStore.getState().scheme).toBe('light');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('init survives a failed preference read: logs it and keeps the default', async () => {
    vi.mocked(api.getPref).mockRejectedValueOnce(new Error('db locked'));
    await useThemeStore.getState().init(fakeSystemScheme(false).mql);
    expect(useThemeStore.getState()).toMatchObject({ preference: 'system', scheme: 'light', isLoading: false });
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('db locked'))).toBe(
      true,
    );
  });

  it('setPreference persists, applies and updates the window chrome', async () => {
    await useThemeStore.getState().init(fakeSystemScheme(false).mql);

    await useThemeStore.getState().setPreference('dark');

    expect(api.setPref).toHaveBeenCalledWith('ui_theme', 'dark');
    expect(useThemeStore.getState()).toMatchObject({ preference: 'dark', scheme: 'dark' });
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(setNativeTheme).toHaveBeenLastCalledWith('dark');
  });

  it('setPreference reverts and rethrows when the preference cannot be written', async () => {
    await useThemeStore.getState().init(fakeSystemScheme(false).mql);
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('read-only'));

    await expect(useThemeStore.getState().setPreference('dark')).rejects.toThrow('read-only');

    expect(useThemeStore.getState()).toMatchObject({ preference: 'system', scheme: 'light' });
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('a native chrome failure is logged, never thrown at the caller', async () => {
    setNativeTheme.mockRejectedValueOnce(new Error('no permission'));
    await useThemeStore.getState().init(fakeSystemScheme(false).mql);
    await expect(useThemeStore.getState().setPreference('dark')).resolves.toBeUndefined();
    expect(useThemeStore.getState().scheme).toBe('dark');
  });

  it('init works without matchMedia (jsdom, old webviews): treats the OS as light', async () => {
    await useThemeStore.getState().init(null);
    expect(useThemeStore.getState()).toMatchObject({ preference: 'system', scheme: 'light', isLoading: false });
  });
});
