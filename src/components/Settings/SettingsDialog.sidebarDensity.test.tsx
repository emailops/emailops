// Settings → Appearance → Sidebar density: shows the density it is given and
// hands a new choice back to its owner (App), logging it to the output panel.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useLogStore } from '@/stores/logStore';
import { SettingsDialog } from './SettingsDialog';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/i18n', () => ({ useUiLanguage: () => ({ language: 'en', setLanguage: vi.fn(), options: [] }) }));
vi.mock('@/stores/aiStore', () => ({ useAiStore: () => ({ enabled: true, setEnabled: vi.fn() }) }));
vi.mock('@/components/shared/LanguageSelect', () => ({ LanguageSelect: () => null }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
// The real Select renders a native <select> on macOS; every other backend call
// resolves to null.
vi.mock('@/lib/api', () => {
  const fns = new Map<PropertyKey, unknown>([['currentPlatform', () => 'macos']]);
  return new Proxy(
    {},
    {
      has: () => true,
      get: (_t, key) => {
        if (key === 'then' || key === '__esModule') return undefined;
        if (!fns.has(key))
          fns.set(
            key,
            vi.fn(async () => null),
          );
        return fns.get(key);
      },
    },
  );
});
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

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  useLogStore.setState({ entries: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function renderDialog(onChangeSidebarDensity: (d: 'comfortable' | 'compact') => void) {
  act(() =>
    root.render(
      <SettingsDialog
        activeAccountId={null}
        currentLayout="split"
        onChangeLayout={() => {}}
        sidebarDensity="comfortable"
        onChangeSidebarDensity={onChangeSidebarDensity}
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
}

function densitySelect(): HTMLSelectElement {
  const select = container.querySelector<HTMLSelectElement>('[data-testid="sidebar-density-select"]');
  if (!select) throw new Error('sidebar density select not rendered');
  return select;
}

describe('SettingsDialog sidebar density', () => {
  it('shows the density it is given on the Appearance tab', () => {
    renderDialog(() => {});

    expect(densitySelect().value).toBe('comfortable');
  });

  it('hands a new choice back to its owner', () => {
    const onChange = vi.fn();
    renderDialog(onChange);

    act(() => {
      densitySelect().value = 'compact';
      densitySelect().dispatchEvent(new Event('change', { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith('compact');
  });

  it('logs the change to the output panel', () => {
    renderDialog(() => {});

    act(() => {
      densitySelect().value = 'compact';
      densitySelect().dispatchEvent(new Event('change', { bubbles: true }));
    });

    const { entries } = useLogStore.getState();
    const entry = entries[entries.length - 1];
    expect(entry?.level).toBe('info');
    expect(entry?.source).toBe('system');
    expect(entry?.message).toBe('Sidebar density: compact');
  });
});
