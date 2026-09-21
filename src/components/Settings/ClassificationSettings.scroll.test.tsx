// Regression: the classification settings body must NOT remount on re-render.
//
// Same defect `AiSettings.scroll.test.tsx` covers: `Shell` (the modal/embedded
// wrapper) was defined INSIDE the ClassificationSettings function body, so
// every render produced a new component identity. React treats a changed
// component type as a different component and unmounts + remounts the whole
// subtree. Reported from Windows during a large initial Gmail sync: each
// `sync-progress` batch event re-renders App → SettingsDialog → this panel,
// so the window flickered and the user-dragged height of the intents/topics
// textareas (DOM state, not React state) snapped back to `rows={4}`.
//
// The test proves the structural property directly: capture the scroll
// container node, force a re-render, and assert the SAME DOM node is still
// mounted (a remount would hand back a fresh element).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: vi.fn() }),
}));

vi.mock('./PromptEditorBlock', () => ({
  PromptEditorBlock: () => null,
}));

vi.mock('./ClassificationRulesTab', () => ({
  ClassificationRulesTab: () => null,
}));

vi.mock('@/lib/api', () => ({
  getClassificationConfig: vi.fn(() =>
    Promise.resolve({
      enabled: true,
      categories: ['primary'],
      intents: ['request', 'question'],
      topics: ['billing', 'project'],
    }),
  ),
  setClassificationConfig: vi.fn(() => Promise.resolve()),
  classifyPreviousEmails: vi.fn(() => Promise.resolve()),
  reclassifyAllEmails: vi.fn(() => Promise.resolve()),
}));

import { ClassificationSettings } from './ClassificationSettings';

describe('ClassificationSettings — scroll container stability', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it('keeps the same scroll container and textarea DOM nodes across a re-render', async () => {
    await act(async () => {
      root.render(<ClassificationSettings activeAccountId="acct-1" />);
    });
    // Drain loadConfig()'s await so the config-loaded tree renders.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    const shellBefore = container.querySelector('.overflow-y-auto');
    const intentsBefore = container.querySelectorAll('textarea')[0];
    expect(shellBefore, 'scroll container should be mounted after load').not.toBeNull();
    expect(intentsBefore, 'intents textarea should be mounted after load').toBeDefined();

    // Force a re-render — what every sync-progress batch event does while
    // Settings is open. With module-scoped chrome React reconciles in place;
    // with a render-body wrapper it remounts and these become different nodes.
    await act(async () => {
      root.render(<ClassificationSettings activeAccountId="acct-1" />);
    });

    expect(container.querySelector('.overflow-y-auto')).toBe(shellBefore);
    expect(container.querySelectorAll('textarea')[0]).toBe(intentsBefore);
  });
});
