// Settings → Appearance: the keyboard-shortcuts switch, stored in SQLite.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { SHORTCUTS_ENABLED_PREF, useShortcutStore } from '@/stores/shortcutStore';
import { KeyboardShortcutsSetting } from './KeyboardShortcutsSetting';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useShortcutStore.setState({ enabled: true, helpOpen: false });
  vi.mocked(api.setPref).mockReset();
  vi.mocked(api.setPref).mockResolvedValue(undefined);
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
    root.render(<KeyboardShortcutsSetting />);
  });
}

const toggle = () => container.querySelector('[role="switch"]') as HTMLButtonElement;

describe('KeyboardShortcutsSetting', () => {
  it('shows the current state and mentions the ? key', async () => {
    await render();
    expect(toggle().getAttribute('aria-checked')).toBe('true');
    expect(container.textContent).toContain('Press ?');
  });

  it('turning it off stores the preference', async () => {
    await render();
    await act(async () => toggle().click());
    expect(api.setPref).toHaveBeenCalledWith(SHORTCUTS_ENABLED_PREF, 'false');
    expect(toggle().getAttribute('aria-checked')).toBe('false');
  });

  it('a failed save shows an error and keeps the switch where it was', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await render();
    await act(async () => toggle().click());
    expect(toggle().getAttribute('aria-checked')).toBe('true');
    expect(container.textContent).toContain('disk full');
  });

  it('the list button opens the help overlay', async () => {
    await render();
    const button = container.querySelector('[data-testid="keyboard-shortcuts-show"]') as HTMLButtonElement;
    act(() => button.click());
    expect(useShortcutStore.getState().helpOpen).toBe(true);
  });
});
