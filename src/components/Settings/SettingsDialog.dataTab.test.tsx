// Where the data folder is and its backups have a Settings tab of their own,
// apart from the password and remote-content switches of Privacy & Security.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsDialog, type SettingsTab } from './SettingsDialog';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/i18n', () => ({ useUiLanguage: () => ({ language: 'en', setLanguage: vi.fn(), options: [] }) }));
vi.mock('@/stores/aiStore', () => ({ useAiStore: () => ({ enabled: true, setEnabled: vi.fn() }) }));
vi.mock('@/components/shared/LanguageSelect', () => ({ LanguageSelect: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => {}) }));
vi.mock('./TrustedSendersSection', () => ({ TrustedSendersSection: () => null }));
vi.mock('@/components/DataLocation/DataLocationSection', () => ({
  DataLocationSection: () => <section>data-location-section</section>,
}));
vi.mock('./BackupsSection', () => ({ BackupsSection: () => <section>backups-section</section> }));
vi.mock('@/lib/api', () => ({
  hasMainPassword: vi.fn(async () => false),
  getPref: vi.fn(async () => 'false'),
  setPref: vi.fn(async () => {}),
}));

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function openOn(tab: SettingsTab) {
  await act(async () =>
    root.render(
      <SettingsDialog
        initialTab={tab}
        activeAccountId={null}
        currentLayout="split"
        onChangeLayout={() => {}}
        sidebarDensity="comfortable"
        onChangeSidebarDensity={() => {}}
        tasksEnabled={false}
        onChangeTasksEnabled={() => {}}
        memoriesEnabled={false}
        onChangeMemoriesEnabled={() => {}}
        lensesEnabled={false}
        onChangeLensesEnabled={() => {}}
        onClose={() => {}}
      />,
    ),
  );
  return container.textContent ?? '';
}

describe('SettingsDialog data tab', () => {
  it('shows the data location and the backups', async () => {
    const text = await openOn('data');

    expect(text).toContain('settings:tabs.data');
    expect(text).toContain('data-location-section');
    expect(text).toContain('backups-section');
  });

  it('leaves them out of Privacy & Security', async () => {
    const text = await openOn('privacy');

    expect(text).toContain('settings:privacy.usePassword');
    expect(text).not.toContain('data-location-section');
    expect(text).not.toContain('backups-section');
  });
});
