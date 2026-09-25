// Lenses left the experimental stage: their Settings tab no longer carries
// the Experimental badge that Tasks and Memory still show.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsDialog } from './SettingsDialog';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/i18n', () => ({ useUiLanguage: () => ({ language: 'en', setLanguage: vi.fn(), options: [] }) }));
vi.mock('@/stores/aiStore', () => ({ useAiStore: () => ({ enabled: true, setEnabled: vi.fn() }) }));
vi.mock('@/components/shared/LanguageSelect', () => ({ LanguageSelect: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
for (const m of [
  './AiDraftsSettings',
  './AiSearchSettings',
  './AiSettings',
  './AiTranslationSettings',
  './CalendarSettings',
  './ClassificationSettings',
  './JunkSettings',
  './LensesSettings',
  './MemorySettings',
  './PrivacySettings',
  './TasksSettings',
]) {
  vi.doMock(m, () => new Proxy({}, { get: () => () => null }));
}

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

function tabButton(id: string): HTMLButtonElement {
  act(() =>
    root.render(
      <SettingsDialog
        activeAccountId={null}
        currentLayout="split"
        onChangeLayout={() => {}}
        tasksEnabled={true}
        onChangeTasksEnabled={() => {}}
        memoriesEnabled={true}
        onChangeMemoriesEnabled={() => {}}
        lensesEnabled={true}
        onChangeLensesEnabled={() => {}}
        onClose={() => {}}
      />,
    ),
  );
  const button = [...container.querySelectorAll('button')].find((b) =>
    b.textContent?.startsWith(`settings:tabs.${id}settings:`),
  );
  if (!button) throw new Error(`no ${id} tab`);
  return button;
}

describe('SettingsDialog experimental badges', () => {
  it('shows no Experimental badge on the Lenses tab', () => {
    expect(tabButton('lenses').textContent).not.toContain('settings:dialog.experimental');
  });

  it('keeps the badge on Tasks, which is still experimental', () => {
    expect(tabButton('tasks').textContent).toContain('settings:dialog.experimental');
  });
});
