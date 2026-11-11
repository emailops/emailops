// Settings → Appearance → Theme: three choices that take effect at once and
// persist as the `ui_theme` preference.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));
vi.mock('@/lib/nativeTheme', () => ({ setNativeTheme: vi.fn(async () => {}) }));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { initialThemeState, useThemeStore } from '@/stores/themeStore';
import { ThemeSetting } from './ThemeSetting';

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  useThemeStore.setState({ ...initialThemeState, isLoading: false });
  useLogStore.setState({ entries: [] });
  document.documentElement.classList.remove('dark');
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(<ThemeSetting />);
  });
}

function option(label: string): HTMLButtonElement {
  const el = [...container.querySelectorAll<HTMLButtonElement>('button[role="radio"]')].find((b) =>
    b.textContent?.includes(label),
  );
  if (!el) throw new Error(`no option ${label}`);
  return el;
}

describe('ThemeSetting', () => {
  it('offers light, dark and system, with the current one checked', async () => {
    await render();
    expect(option('settings:appearance.themeLight').getAttribute('aria-checked')).toBe('false');
    expect(option('settings:appearance.themeDark').getAttribute('aria-checked')).toBe('false');
    expect(option('settings:appearance.themeSystem').getAttribute('aria-checked')).toBe('true');
  });

  it('choosing dark applies it at once and persists it', async () => {
    await render();
    await act(async () => option('settings:appearance.themeDark').click());

    expect(api.setPref).toHaveBeenCalledWith('ui_theme', 'dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(option('settings:appearance.themeDark').getAttribute('aria-checked')).toBe('true');
    expect(option('settings:appearance.themeSystem').getAttribute('aria-checked')).toBe('false');
  });

  it('a failed write puts the previous choice back and reports it in the output panel', async () => {
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('disk full'));
    await render();
    await act(async () => option('settings:appearance.themeDark').click());

    expect(option('settings:appearance.themeSystem').getAttribute('aria-checked')).toBe('true');
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(useLogStore.getState().entries.some((e) => e.level === 'error' && e.message.includes('disk full'))).toBe(
      true,
    );
  });
});
