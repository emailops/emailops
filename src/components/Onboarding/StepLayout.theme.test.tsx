// The onboarding's view step offers the theme next to the inbox layout, with
// "follow the system" selected by default. A choice applies at once and is
// persisted as the `ui_theme` preference.

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
import { initialThemeState, useThemeStore } from '@/stores/themeStore';
import { StepLayout } from './StepLayout';

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  useThemeStore.setState({ ...initialThemeState, isLoading: false });
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
    root.render(<StepLayout currentLayout="split" onChangeLayout={() => {}} onBack={() => {}} onNext={() => {}} />);
  });
}

function option(label: string): HTMLButtonElement {
  const el = [...container.querySelectorAll<HTMLButtonElement>('button[role="radio"]')].find((b) =>
    b.textContent?.includes(label),
  );
  if (!el) throw new Error(`no option ${label}`);
  return el;
}

describe('StepLayout theme choice', () => {
  it('offers the three themes with "system" selected by default', async () => {
    await render();
    expect(container.textContent).toContain('auth:onboarding.layout.themeTitle');
    expect(option('settings:appearance.themeSystem').getAttribute('aria-checked')).toBe('true');
    expect(option('settings:appearance.themeLight').getAttribute('aria-checked')).toBe('false');
    expect(option('settings:appearance.themeDark').getAttribute('aria-checked')).toBe('false');
  });

  it('choosing dark applies it at once and persists it', async () => {
    await render();
    await act(async () => option('settings:appearance.themeDark').click());
    expect(api.setPref).toHaveBeenCalledWith('ui_theme', 'dark');
    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(option('settings:appearance.themeDark').getAttribute('aria-checked')).toBe('true');
  });

  it('keeps the layout cards and the navigation', async () => {
    await render();
    expect(container.textContent).toContain('auth:onboarding.layout.splitTitle');
    expect(container.textContent).toContain('auth:onboarding.layout.continue');
  });
});
