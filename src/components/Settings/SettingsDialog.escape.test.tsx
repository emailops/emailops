// Every other dialog in the app closes on Escape through the shared Modal.
// Settings is a bespoke dialog and silently ignored the key. With a dialog open
// on top of it, the key is that dialog's: it must not throw the panel away.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Modal } from '@/components/common/Modal';
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

describe('SettingsDialog', () => {
  it('closes on Escape', () => {
    const onClose = vi.fn();
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
          onClose={onClose}
        />,
      ),
    );
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('leaves Escape to a dialog open on top of it', async () => {
    const onClose = vi.fn();
    const closeConfirmation = vi.fn();
    await act(async () =>
      root.render(
        <>
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
            onClose={onClose}
          />
          <Modal open onClose={closeConfirmation} title="Remove password">
            <p>Sure?</p>
          </Modal>
        </>,
      ),
    );

    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    });

    expect(closeConfirmation).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
  });
});
