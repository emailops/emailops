// Regression: after a chat model finishes downloading/linking, the row's
// radio never becomes selected unless it happens to be the recommended
// default — because `chatModelId` is seeded once on mount (to the
// recommended model's id, even though it isn't downloaded yet) and nothing
// re-syncs it when a *different* model becomes the actual local one.
//
// This is not just cosmetic: `handleContinue` sends `chatModelId` straight
// to `setAiConfig`, so clicking Continue after linking/downloading a
// non-recommended model as the first local chat model would persist the
// recommended model's id — which was never downloaded and doesn't exist on
// disk — instead of the model the user actually has.
//
// The test proves the fix by asserting what Continue actually submits: the
// model that just finished, not the initial recommended default.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const listeners: Record<string, (event: { payload: unknown }) => void> = {};

// `t` must be a STABLE function identity across renders. StepAiBackend's
// main effect lists `t` in its dependency array; a fresh `t` closure on
// every `useTranslation()` call (as a naive `() => ({ t: (k) => k })` mock
// would produce) makes React tear down and re-run that effect on every
// re-render, unregistering the `model-download-progress` listener right
// after it registers — was mistaken for the listener "never" registering.
const identityT = (key: string) => key;
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: identityT }),
}));

const { listenMock } = vi.hoisted(() => ({ listenMock: vi.fn() }));
listenMock.mockImplementation((event: string, cb: (e: { payload: unknown }) => void) => {
  listeners[event] = cb;
  return Promise.resolve(() => {
    delete listeners[event];
  });
});

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}));

vi.mock('@tauri-apps/plugin-shell', () => ({
  open: vi.fn(() => Promise.resolve()),
}));

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: vi.fn(() => Promise.resolve(null)),
}));

// `addLog` must also be a STABLE identity across renders for the same
// reason as `identityT` above — it's another dependency of the effect that
// registers the `model-download-progress` listener.
const { addLogMock } = vi.hoisted(() => ({ addLogMock: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLogMock }) => unknown) => selector({ addLog: addLogMock }),
}));

// `vi.mock` factories are hoisted above the module's top-level `const`s, so
// everything the factory needs — fixtures and spies alike — must be
// declared inside `vi.hoisted`.
const { OTHER_CHAT, BUNDLED_EMBED, setAiConfig, listCatalogModels } = vi.hoisted(() => {
  const recommendedChat = {
    id: 'qwen-recommended',
    displayName: 'Qwen Recommended',
    kind: 'chat' as const,
    sizeBytes: 1,
    contextWindow: 1,
    license: 'apache-2.0',
    minRamGb: 1,
    recommended: true,
    supportsTools: true,
    isLocal: false,
    isLinked: false,
  };
  const otherChat = {
    id: 'qwen-other',
    displayName: 'Qwen Other',
    kind: 'chat' as const,
    sizeBytes: 1,
    contextWindow: 1,
    license: 'apache-2.0',
    minRamGb: 1,
    recommended: false,
    supportsTools: true,
    isLocal: false,
    isLinked: false,
  };
  const bundledEmbed = {
    id: 'embed-bundled',
    displayName: 'Embed Bundled',
    kind: 'embedding' as const,
    sizeBytes: 1,
    contextWindow: 1,
    license: 'apache-2.0',
    minRamGb: 1,
    recommended: true,
    supportsTools: false,
    isLocal: true,
    isLinked: false,
  };
  const catalogBefore = [recommendedChat, otherChat, bundledEmbed];
  const catalogAfter = [recommendedChat, { ...otherChat, isLocal: true, isLinked: true }, bundledEmbed];

  let callCount = 0;
  return {
    OTHER_CHAT: otherChat,
    BUNDLED_EMBED: bundledEmbed,
    setAiConfig: vi.fn(() => Promise.resolve()),
    listCatalogModels: vi.fn(() => {
      callCount += 1;
      return Promise.resolve(callCount === 1 ? catalogBefore : catalogAfter);
    }),
  };
});

vi.mock('@/lib/api', () => ({
  getAiConfig: vi.fn(() =>
    Promise.resolve({
      provider: 'llamacpp',
      model: '',
      embeddingModel: '',
      monthlyBudgetUsd: 0,
      hasApiKey: false,
      thinkingEnabled: false,
    }),
  ),
  listCatalogModels,
  setAiConfig,
  linkLocalModel: vi.fn(() => Promise.resolve()),
  startModelDownload: vi.fn(() => Promise.resolve()),
  cancelModelDownload: vi.fn(() => Promise.resolve()),
  testAiProvider: vi.fn(() => Promise.resolve()),
}));

import { StepAiBackend } from './StepAiBackend';

describe('StepAiBackend — selecting the model that just finished', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    listCatalogModels.mockClear();
    setAiConfig.mockClear();
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  // Real timers throughout: `vi.useFakeTimers()` also freezes React's own
  // passive-effect scheduling under jsdom (it falls back to a timer-based
  // scheduler here), so the mount effect that registers the
  // `model-download-progress` listener never runs. Flushing with a real,
  // short `setTimeout` avoids that trap — same approach as
  // AiSettings.scroll.test.tsx.
  const flush = (ms = 0) => act(async () => new Promise((resolve) => setTimeout(resolve, ms)));

  it('submits the newly-linked non-recommended model on Continue, not the never-downloaded recommended default', async () => {
    await act(async () => {
      root.render(<StepAiBackend onBack={() => {}} onNext={() => {}} />);
    });
    // Flush the mount effect's chained awaits (getAiConfig → listCatalogModels).
    await flush();

    // Backend finishes linking the NON-recommended model as the first local
    // chat model (mirrors model_manager::link_local_model's auto-select).
    expect(Object.keys(listeners), 'listener must be registered by now').toContain('model-download-progress');
    await act(async () => {
      listeners['model-download-progress']?.({
        payload: { modelId: OTHER_CHAT.id, downloadedBytes: 0, totalBytes: 0, status: 'complete' },
      });
    });

    // The completion handler debounces its catalog refetch by 400ms.
    await flush(450);

    const continueButton = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent === 'auth:onboarding.aiBackend.continue',
    );
    expect(continueButton, 'Continue button must be present').toBeTruthy();

    await act(async () => {
      continueButton?.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(setAiConfig).toHaveBeenCalledWith('llamacpp', OTHER_CHAT.id, BUNDLED_EMBED.id, null, 0, false);
  });
});
