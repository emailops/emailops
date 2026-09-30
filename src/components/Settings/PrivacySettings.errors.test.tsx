// Privacy settings must not hang on its loading state when the initial read
// fails, and a remote-content toggle that fails to save must snap back and say
// so rather than showing a setting that is not in effect.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => {}) }));
vi.mock('./TrustedSendersSection', () => ({ TrustedSendersSection: () => null }));
vi.mock('@/lib/api', () => ({
  hasMainPassword: vi.fn(async () => false),
  getPref: vi.fn(async () => 'false'),
  setPref: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { PrivacySettings } from './PrivacySettings';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useLogStore.setState({ entries: [] });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => {
    root.render(<PrivacySettings />);
  });
}

const remoteToggle = () =>
  [...container.querySelectorAll<HTMLButtonElement>('button[role="switch"]')].find((b) =>
    b.closest('div.flex')?.textContent?.includes('settings:privacy.allowRemoteToggle'),
  );

describe('PrivacySettings error handling', () => {
  it('shows an error instead of loading forever when the settings cannot be read', async () => {
    vi.mocked(api.hasMainPassword).mockRejectedValueOnce(new Error('db locked'));
    await mount();

    expect(container.textContent).not.toContain('settings:privacy.loading');
    expect(container.textContent).toContain('settings:privacy.loadFailed');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });

  it('rolls the remote-content toggle back and says so when saving fails', async () => {
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('db locked'));
    await mount();
    const toggle = remoteToggle();
    if (!toggle) throw new Error('remote toggle not rendered');

    await act(async () => toggle.click());

    expect(remoteToggle()?.getAttribute('aria-checked')).toBe('false');
    expect(container.textContent).toContain('settings:privacy.saveFailed');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });
});
