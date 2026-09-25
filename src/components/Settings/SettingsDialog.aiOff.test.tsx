// With the master AI switch off, Settings keeps only the tabs that work
// without a model. Junk filtering is one of them — the detector is a local,
// model-free scorer that runs on every sync regardless of the AI switch — but
// its tab was hidden anyway, so a plain-email-client user could not choose
// what happens to flagged mail.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsDialog } from './SettingsDialog';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/i18n', () => ({ useUiLanguage: () => ({ language: 'en', setLanguage: vi.fn(), options: [] }) }));
vi.mock('@/stores/aiStore', () => ({ useAiStore: () => ({ enabled: false, setEnabled: vi.fn() }) }));
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

function renderWithAiOff() {
  act(() =>
    root.render(
      <SettingsDialog
        activeAccountId={null}
        currentLayout="split"
        onChangeLayout={() => {}}
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

describe('SettingsDialog with AI switched off', () => {
  it('keeps the Junk tab, because junk filtering uses no model', () => {
    expect(renderWithAiOff()).toContain('settings:tabs.junk');
  });

  it('still hides the tabs that need a model', () => {
    const text = renderWithAiOff();
    for (const tab of ['classification', 'aidrafts', 'aisearch', 'tasks', 'memory', 'lenses', 'aitranslation']) {
      expect(text).not.toContain(`settings:tabs.${tab}`);
    }
  });
});
