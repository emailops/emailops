// Regression: the AI settings body must NOT remount on re-render.
//
// `Shell` (the modal/embedded wrapper) was defined INSIDE the AiSettings
// function body, so every render produced a new component identity. React
// treats a changed component type as a different component and unmounts +
// remounts the whole subtree — including the `overflow-y-auto` scroll
// container. That reset the user's scroll position on every state change
// (reported when nudging the context-window number spinner: the page jumped
// back to the top after each click). Hoisting `Shell` to module scope keeps
// its identity stable so React reconciles in place and the scroll node — and
// the scroll position with it — survives.
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

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock('@/stores/aiStore', () => ({
  useAiStore: () => ({ enabled: true, setEnabled: vi.fn() }),
}));

vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: vi.fn() }),
}));

vi.mock('@/lib/api', () => ({
  getAiConfig: vi.fn(() =>
    Promise.resolve({
      provider: 'llamacpp',
      model: 'qwen3.5-4b',
      embeddingModel: 'nomic-embed-text',
      monthlyBudgetUsd: 0,
      hasApiKey: false,
      thinkingEnabled: false,
    }),
  ),
  // Probed on mount to decide whether the embedded provider tab is selectable
  // (false on Intel Macs, whose GPU cannot run the Metal kernels).
  detectAiCapability: vi.fn(() =>
    Promise.resolve({
      appleSilicon: true,
      localAiCapable: true,
      embeddedAiAvailable: true,
      totalRamGb: 32,
      minRamGbForLocalAi: 8,
      os: 'macos',
      arch: 'aarch64',
    }),
  ),
  listCatalogModels: vi.fn(() => Promise.resolve([])),
  listOllamaModels: vi.fn(() => Promise.resolve([])),
  getPref: vi.fn(() => Promise.resolve(null)),
  setPref: vi.fn(() => Promise.resolve()),
  setAiConfig: vi.fn(() => Promise.resolve()),
  testAiProvider: vi.fn(() => Promise.resolve('')),
  startModelDownload: vi.fn(() => Promise.resolve()),
  cancelModelDownload: vi.fn(() => Promise.resolve()),
  deleteLocalModel: vi.fn(() => Promise.resolve()),
  regenerateEmbeddings: vi.fn(() => Promise.resolve()),
  currentPlatform: vi.fn(() => ''),
}));

import { AiSettings } from './AiSettings';

describe('AiSettings — scroll container stability', () => {
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

  it('keeps the same scroll container DOM node across a re-render', async () => {
    // Mount and let the async loadAll() settle so the main (config-loaded)
    // tree with the scroll container renders.
    await act(async () => {
      root.render(<AiSettings embedded onClose={() => {}} />);
    });
    // Drain loadAll()'s chained awaits (getAiConfig → catalog → prefs) in one
    // macrotask flush so the config-loaded tree with the scroll container renders.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    const before = container.querySelector('.overflow-y-auto');
    expect(before, 'scroll container should be mounted after load').not.toBeNull();

    // Force a parent re-render of AiSettings (new onClose identity → new
    // element). With Shell hoisted this reconciles in place; with Shell defined
    // inline it remounts the subtree and `after` becomes a different node.
    await act(async () => {
      root.render(<AiSettings embedded onClose={() => {}} />);
    });

    const after = container.querySelector('.overflow-y-auto');
    expect(after).toBe(before);
  });
});
