// The embedding model preference is shared by every provider, and an
// OpenRouter embedding model must pass the backend's dimension probe before it
// is saved: a model of another vector size cannot fill the email index, and a
// local model id sent to OpenRouter fails on every email.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: { error?: string }) => (vars?.error ? `${key}: ${vars.error}` : key),
  }),
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

vi.mock('@/stores/featureToggleStore', () => ({
  useHelpDocsEnabledStore: () => ({ enabled: true, setEnabled: vi.fn(() => Promise.resolve()) }),
}));

vi.mock('./AiSettings/UsageSummary', () => ({ UsageSummary: () => null }));
vi.mock('./AiSettings/ChatPromptsSection', () => ({ ChatPromptsSection: () => null }));

const api = vi.hoisted(() => ({
  getAiConfig: vi.fn(),
  detectAiCapability: vi.fn(() => Promise.resolve({ embeddedAiAvailable: true })),
  listCatalogModels: vi.fn(() =>
    Promise.resolve([
      {
        id: 'chat-local-gguf',
        displayName: 'Local chat',
        kind: 'chat',
        sizeBytes: 1,
        contextWindow: 2048,
        license: 'test',
        minRamGb: 1,
        recommended: true,
        supportsTools: true,
        isLocal: true,
        isLinked: false,
      },
      {
        id: 'embed-local-gguf',
        displayName: 'Local embed',
        kind: 'embedding',
        sizeBytes: 1,
        contextWindow: 2048,
        license: 'test',
        minRamGb: 1,
        recommended: true,
        supportsTools: false,
        isLocal: true,
        isLinked: false,
      },
    ]),
  ),
  listOllamaModels: vi.fn((): Promise<string[]> => Promise.resolve([])),
  listAiEmbeddingModels: vi.fn(() =>
    Promise.resolve([
      { id: 'vendor/embed', name: 'Vendor Embed', pricing: { prompt: 0, completion: 0, request: 0 } },
      { id: 'vendor/embed-large', name: 'Vendor Embed Large', pricing: { prompt: 0, completion: 0, request: 0 } },
    ]),
  ),
  validateOpenRouterEmbeddingModel: vi.fn(() => Promise.resolve()),
  getAutoNCtx: vi.fn(() => Promise.resolve(8192)),
  getPref: vi.fn(() => Promise.resolve(null)),
  setPref: vi.fn(() => Promise.resolve()),
  setAiConfig: vi.fn(() => Promise.resolve()),
  regenerateEmbeddings: vi.fn(() => Promise.resolve()),
  currentPlatform: vi.fn(() => 'macos'),
}));
vi.mock('@/lib/api', () => api);

import { AiSettings } from './AiSettings';

function savedConfig(over: Record<string, unknown>) {
  return {
    provider: 'openrouter',
    model: 'vendor/model',
    embeddingModel: 'vendor/embed',
    embeddingModelValidated: true,
    monthlyBudgetUsd: 0,
    periodStart: 0,
    hasApiKey: true,
    thinkingEnabled: false,
    zeroDataRetention: false,
    ...over,
  };
}

describe('AiSettings — embedding model', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    vi.clearAllMocks();
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  async function settle() {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }

  async function mount(config: Record<string, unknown>) {
    api.getAiConfig.mockResolvedValue(savedConfig(config));
    await act(async () => {
      root.render(<AiSettings />);
    });
    await settle();
  }

  function embeddingSelect(): HTMLSelectElement {
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="settings:ai.embeddingModel"]');
    if (!select) throw new Error('embedding model selector not rendered');
    return select;
  }

  function button(label: string): HTMLButtonElement {
    const found = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes(label));
    if (!found) throw new Error(`button ${label} not rendered`);
    return found;
  }

  function chatModelInput(): HTMLInputElement {
    const input = container.querySelector<HTMLInputElement>(
      'input[placeholder="settings:openRouter.chatModelPlaceholder"]',
    );
    if (!input) throw new Error('OpenRouter chat model field not rendered');
    return input;
  }

  async function typeChatModel(model: string) {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    act(() => {
      setValue?.call(chatModelInput(), model);
      chatModelInput().dispatchEvent(new Event('input', { bubbles: true }));
    });
    await settle();
  }

  async function switchTo(providerLabel: string) {
    await act(async () => {
      button(providerLabel).click();
    });
    await settle();
  }

  async function choose(model: string) {
    act(() => {
      embeddingSelect().value = model;
      embeddingSelect().dispatchEvent(new Event('change', { bubbles: true }));
    });
    await settle();
  }

  async function save() {
    await act(async () => {
      button('common:actions.save').click();
    });
    await settle();
  }

  const reindexDialogShown = () => container.textContent?.includes('settings:confirmReindex.title') ?? false;

  async function confirmReindex() {
    await act(async () => {
      button('settings:confirmReindex.confirm').click();
    });
    await settle();
  }

  it('lists the OpenRouter embedding models in the selector', async () => {
    await mount({});
    expect(api.listAiEmbeddingModels).toHaveBeenCalledWith('openrouter');
    expect(Array.from(embeddingSelect().options).map((o) => o.value)).toEqual([
      '',
      'vendor/embed',
      'vendor/embed-large',
    ]);
  });

  it('checks a newly chosen OpenRouter embedding model before saving it, then re-indexes', async () => {
    await mount({});
    await choose('vendor/embed-large');
    await save();
    await confirmReindex();

    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith('vendor/embed-large', null, false);
    expect(api.setAiConfig).toHaveBeenCalledWith(
      'openrouter',
      'vendor/model',
      'vendor/embed-large',
      null,
      0,
      false,
      false,
    );
    expect(api.validateOpenRouterEmbeddingModel.mock.invocationCallOrder[0]).toBeLessThan(
      api.setAiConfig.mock.invocationCallOrder[0],
    );
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(1);
  });

  it('does not save when the model fails the check, and shows why', async () => {
    api.validateOpenRouterEmbeddingModel.mockRejectedValueOnce('returns 1536-dimension vectors');
    await mount({});
    await choose('vendor/embed-large');
    await save();
    await confirmReindex();

    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(api.regenerateEmbeddings).not.toHaveBeenCalled();
    expect(container.textContent).toContain('settings:openRouter.embeddingCheckFailed: returns 1536-dimension vectors');
  });

  it('does not check again a model that is saved and already validated', async () => {
    await mount({});
    await save();
    expect(reindexDialogShown()).toBe(false);
    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig).toHaveBeenCalled();
    expect(api.regenerateEmbeddings).not.toHaveBeenCalled();
  });

  it('checks on save a model that was saved without ever being validated', async () => {
    await mount({ embeddingModelValidated: false });
    await save();
    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith('vendor/embed', null, false);
  });

  it('switching to OpenRouter drops the local embedding model instead of sending its id', async () => {
    await mount({ provider: 'llamacpp', model: 'chat-local-gguf', embeddingModel: 'embed-local-gguf' });
    await switchTo('settings:ai.providerOpenRouterLabel');

    expect(embeddingSelect().value).toBe('');
    await typeChatModel('vendor/model');
    await save();
    await confirmReindex();

    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['openrouter', 'vendor/model', '']);
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(1);
  });

  it('switching away from OpenRouter picks a model the new provider can run', async () => {
    await mount({});
    await switchTo('settings:ai.providerEmbeddedLabel');
    await save();
    await confirmReindex();

    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['llamacpp', 'chat-local-gguf', 'embed-local-gguf']);
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(1);
  });

  it('switching to OpenRouter empties the chat model instead of showing the in-app one', async () => {
    await mount({ provider: 'llamacpp', model: 'chat-local-gguf', embeddingModel: 'embed-local-gguf' });
    await switchTo('settings:ai.providerOpenRouterLabel');

    expect(chatModelInput().value).toBe('');
  });

  it('returning to the saved provider restores its chat model', async () => {
    await mount({});
    await switchTo('settings:ai.providerEmbeddedLabel');
    await switchTo('settings:ai.providerOpenRouterLabel');

    expect(chatModelInput().value).toBe('vendor/model');
  });

  it('switching to Ollama picks the first chat model Ollama has', async () => {
    api.listOllamaModels.mockResolvedValueOnce(['ollama-chat', 'nomic-embed-text']);
    await mount({});
    await switchTo('settings:ai.providerOllamaLabel');
    await save();
    await confirmReindex();

    expect(api.setAiConfig.mock.calls[0].slice(0, 2)).toEqual(['ollama', 'ollama-chat']);
  });

  it('does not save OpenRouter without a chat model, and says so', async () => {
    await mount({ provider: 'llamacpp', model: 'chat-local-gguf', embeddingModel: 'embed-local-gguf' });
    await switchTo('settings:ai.providerOpenRouterLabel');
    await save();

    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(api.regenerateEmbeddings).not.toHaveBeenCalled();
    expect(container.textContent).toContain('settings:openRouter.chatModelRequired');
    expect(reindexDialogShown()).toBe(false);
  });

  it('asks before replacing the Embeddings, and does nothing until answered', async () => {
    await mount({});
    await choose('vendor/embed-large');
    await save();

    expect(reindexDialogShown()).toBe(true);
    expect(container.textContent).toContain('settings:confirmReindex.body');
    expect(container.textContent).toContain('settings:confirmReindex.openRouter');
    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(api.regenerateEmbeddings).not.toHaveBeenCalled();
  });

  it('cancelling the question saves nothing and keeps the form as it was', async () => {
    await mount({});
    await choose('vendor/embed-large');
    await save();
    await act(async () => {
      button('common:actions.cancel').click();
    });
    await settle();

    expect(reindexDialogShown()).toBe(false);
    expect(embeddingSelect().value).toBe('vendor/embed-large');
    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(api.regenerateEmbeddings).not.toHaveBeenCalled();
  });

  it('says semantic search will be off when the new choice is no model', async () => {
    await mount({});
    await choose('');
    await save();

    expect(container.textContent).toContain('settings:confirmReindex.bodyNone');
    expect(container.textContent).not.toContain('settings:confirmReindex.openRouter');
    await act(async () => {
      button('settings:confirmReindex.confirmNone').click();
    });
    await settle();
    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['openrouter', 'vendor/model', '']);
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(1);
  });

  it('does not mention OpenRouter when the new model runs locally', async () => {
    await mount({});
    await switchTo('settings:ai.providerEmbeddedLabel');
    await save();

    expect(container.textContent).toContain('settings:confirmReindex.body');
    expect(container.textContent).not.toContain('settings:confirmReindex.openRouter');
  });

  it('does not ask when there were no Embeddings to replace', async () => {
    await mount({ embeddingModel: '' });
    await choose('vendor/embed');
    await save();

    expect(reindexDialogShown()).toBe(false);
    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith('vendor/embed', null, false);
    expect(api.setAiConfig).toHaveBeenCalled();
  });
});
