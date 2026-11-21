// Settings → Privacy & Security: the daily update check can be turned off. It
// is on unless the stored preference says "false".

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
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  setMainPassword: vi.fn(async () => {}),
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
  // clearAllMocks keeps implementations: start every test with nothing stored.
  vi.mocked(api.getPref).mockResolvedValue(null);
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

const updateToggle = () =>
  [...container.querySelectorAll<HTMLButtonElement>('button[role="switch"]')].find((b) =>
    b.closest('div.flex')?.textContent?.includes('settings:privacy.updateCheckToggle'),
  );

describe('PrivacySettings update check', () => {
  it('is on when no preference is stored', async () => {
    await mount();

    expect(updateToggle()?.getAttribute('aria-checked')).toBe('true');
  });

  it('is off when the stored preference is "false"', async () => {
    vi.mocked(api.getPref).mockImplementation(async (key: string) =>
      key === 'app_update_check_enabled' ? 'false' : null,
    );
    await mount();

    expect(updateToggle()?.getAttribute('aria-checked')).toBe('false');
  });

  it('stores "false" when turned off', async () => {
    await mount();

    await act(async () => updateToggle()?.click());

    expect(api.setPref).toHaveBeenCalledWith('app_update_check_enabled', 'false');
    expect(updateToggle()?.getAttribute('aria-checked')).toBe('false');
  });

  it('rolls back and says so when saving fails', async () => {
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('db locked'));
    await mount();

    await act(async () => updateToggle()?.click());

    expect(updateToggle()?.getAttribute('aria-checked')).toBe('true');
    expect(container.textContent).toContain('settings:privacy.saveFailed');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });
});
