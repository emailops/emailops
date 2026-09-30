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

const addLog = vi.hoisted(() => vi.fn());
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLog }) => unknown) => selector({ addLog }),
}));

vi.mock('@/stores/featureToggleStore', () => ({
  useHelpDocsEnabledStore: () => ({ enabled: true, setEnabled: vi.fn(() => Promise.resolve()) }),
}));

vi.mock('./AiSettings/UsageSummary', () => ({ UsageSummary: () => null }));
vi.mock('./AiSettings/ChatPromptsSection', () => ({ ChatPromptsSection: () => null }));
// The recommended list is the panel's own concern (OpenRouterPanel.test).
vi.mock('./AiSettings/openRouterEmbeddingModels', () => ({ RECOMMENDED_OPENROUTER_EMBEDDING_MODELS: [] }));

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
  getAiProviderActivity: vi.fn(
    (): Promise<{ provider: string; items: unknown[] }> => Promise.resolve({ provider: 'openrouter', items: [] }),
  ),
  cancelAiProviderWork: vi.fn(() => Promise.resolve(1)),
  currentPlatform: vi.fn(() => 'macos'),
}));
vi.mock('@/lib/api', () => api);

import { AiSettings } from './AiSettings';
import { DEFAULT_OPENROUTER_CHAT_MODEL } from './AiSettings/helpers';

const NOTHING = { model: null, embeddingModel: null };

// What `get_ai_config` answers: the saved provider's models are its remembered
// ones, and a saved OpenRouter embedding model has passed the probe.
function savedConfig(over: Record<string, unknown>) {
  const base = {
    provider: 'openrouter',
    model: 'vendor/model',
    embeddingModel: 'vendor/embed',
    monthlyBudgetUsd: 0,
    periodStart: 0,
    hasApiKey: true,
    thinkingEnabled: false,
    zeroDataRetention: false,
    ...over,
  };
  return {
    openRouterValidatedEmbeddingModel: base.provider === 'openrouter' ? base.embeddingModel : null,
    ...base,
    remembered: {
      llamacpp: NOTHING,
      ollama: NOTHING,
      openrouter: NOTHING,
      [base.provider]: { model: base.model, embeddingModel: base.embeddingModel },
      ...(over.remembered as object | undefined),
    },
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

  const dataPolicyRejection = {
    code: 'ai_data_policy',
    params: { model: 'vendor/embed-large' },
    message: 'The model vendor/embed-large is not available under the data policy',
  };

  it('says zero data retention is what blocks the model, and what the options are', async () => {
    api.validateOpenRouterEmbeddingModel.mockRejectedValueOnce(dataPolicyRejection);
    await mount({ zeroDataRetention: true });
    await choose('vendor/embed-large');
    await save();
    await confirmReindex();

    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith('vendor/embed-large', null, true);
    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(container.textContent).toContain('settings:openRouter.embeddingZdrBlocked');
    expect(container.textContent).not.toContain('settings:openRouter.embeddingCheckFailed');
  });

  it('keeps the general message for a data-policy refusal without zero data retention', async () => {
    api.validateOpenRouterEmbeddingModel.mockRejectedValueOnce(dataPolicyRejection);
    await mount({});
    await choose('vendor/embed-large');
    await save();
    await confirmReindex();

    expect(container.textContent).toContain('settings:openRouter.embeddingCheckFailed');
    expect(container.textContent).not.toContain('settings:openRouter.embeddingZdrBlocked');
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
    await mount({ openRouterValidatedEmbeddingModel: null });
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

  it('switching to OpenRouter offers its default chat model instead of the in-app one', async () => {
    await mount({ provider: 'llamacpp', model: 'chat-local-gguf', embeddingModel: 'embed-local-gguf' });
    await switchTo('settings:ai.providerOpenRouterLabel');

    expect(chatModelInput().value).toBe(DEFAULT_OPENROUTER_CHAT_MODEL);
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
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    await act(async () => {
      setValue?.call(chatModelInput(), '');
      chatModelInput().dispatchEvent(new Event('input', { bubbles: true }));
    });
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

  // The developer's sequence: OpenRouter with an embedding model, then the
  // in-app provider, then back. The preferences are shared by every provider,
  // so only what is remembered per provider can bring the choice back.
  it('brings back the OpenRouter models after another provider was saved, without a second check', async () => {
    await mount({});
    await switchTo('settings:ai.providerEmbeddedLabel');
    // The backend's answer once the in-app provider is saved.
    api.getAiConfig.mockResolvedValue(
      savedConfig({
        provider: 'llamacpp',
        model: 'chat-local-gguf',
        embeddingModel: 'embed-local-gguf',
        openRouterValidatedEmbeddingModel: 'vendor/embed',
        remembered: { openrouter: { model: 'vendor/model', embeddingModel: 'vendor/embed' } },
      }),
    );
    await save();
    await confirmReindex();
    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['llamacpp', 'chat-local-gguf', 'embed-local-gguf']);

    await switchTo('settings:ai.providerOpenRouterLabel');

    expect(embeddingSelect().value).toBe('vendor/embed');
    expect(chatModelInput().value).toBe('vendor/model');
    expect(container.textContent).not.toContain('settings:openRouter.embeddingNeedsCheck');

    await save();
    expect(reindexDialogShown()).toBe(true);
    await confirmReindex();

    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[1].slice(0, 3)).toEqual(['openrouter', 'vendor/model', 'vendor/embed']);
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(2);
  });

  it('does not ask to re-index when returning to the saved provider restores the saved model', async () => {
    await mount({});
    await switchTo('settings:ai.providerEmbeddedLabel');
    await switchTo('settings:ai.providerOpenRouterLabel');
    await save();

    expect(reindexDialogShown()).toBe(false);
    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['openrouter', 'vendor/model', 'vendor/embed']);
    expect(api.regenerateEmbeddings).not.toHaveBeenCalled();
  });

  it('checks a remembered OpenRouter model that is not the one that passed the check', async () => {
    await mount({
      provider: 'llamacpp',
      model: 'chat-local-gguf',
      embeddingModel: 'embed-local-gguf',
      openRouterValidatedEmbeddingModel: 'vendor/embed',
      remembered: { openrouter: { model: 'vendor/model', embeddingModel: 'vendor/embed-large' } },
    });
    await switchTo('settings:ai.providerOpenRouterLabel');
    expect(embeddingSelect().value).toBe('vendor/embed-large');
    await save();
    await confirmReindex();

    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith('vendor/embed-large', null, false);
  });

  // The state a provider switch outside Settings could leave: the in-app
  // provider saved with an OpenRouter embedding model it cannot run.
  it('replaces a saved embedding model the provider cannot use, and asks before re-indexing', async () => {
    await mount({
      provider: 'llamacpp',
      model: 'chat-local-gguf',
      embeddingModel: 'vendor/embed',
      remembered: {
        llamacpp: { model: 'chat-local-gguf', embeddingModel: null },
        openrouter: { model: null, embeddingModel: 'vendor/embed' },
      },
    });
    await save();

    expect(reindexDialogShown()).toBe(true);
    await confirmReindex();
    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['llamacpp', 'chat-local-gguf', 'embed-local-gguf']);
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(1);
  });
  // ── Work in progress ───────────────────────────────────────────────────

  const REBUILD_RUNNING = {
    provider: 'openrouter',
    items: [{ kind: 'embeddingsRebuild', running: true, stopping: false, progress: { current: 40, total: 500 } }],
  };
  const workDialogShown = () => container.textContent?.includes('settings:aiWork.title') ?? false;

  it('asks about the work in progress first, and about the re-index only after it', async () => {
    api.getAiProviderActivity.mockResolvedValueOnce(REBUILD_RUNNING);
    await mount({});
    await choose('vendor/embed-large');
    await save();

    expect(workDialogShown()).toBe(true);
    expect(reindexDialogShown()).toBe(false);
    expect(api.setAiConfig).not.toHaveBeenCalled();

    // Stopping ends the rebuild; the queue is empty on the next read.
    await act(async () => {
      button('settings:aiWork.stop').click();
    });
    await settle();

    expect(api.cancelAiProviderWork).toHaveBeenCalledTimes(1);
    expect(workDialogShown()).toBe(false);
    expect(reindexDialogShown()).toBe(true);
    expect(api.setAiConfig).not.toHaveBeenCalled();

    await confirmReindex();
    expect(api.setAiConfig).toHaveBeenCalledTimes(1);
    expect(api.regenerateEmbeddings).toHaveBeenCalledTimes(1);
  });

  it('saves nothing and keeps the form when the work-in-progress dialog is cancelled', async () => {
    api.getAiProviderActivity.mockResolvedValueOnce(REBUILD_RUNNING);
    await mount({});
    await choose('vendor/embed-large');
    await save();
    await act(async () => {
      button('common:actions.cancel').click();
    });
    await settle();

    expect(workDialogShown()).toBe(false);
    expect(reindexDialogShown()).toBe(false);
    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(api.cancelAiProviderWork).not.toHaveBeenCalled();
    expect(embeddingSelect().value).toBe('vendor/embed-large');
  });

  it('does not ask when the work in progress does not use what changes', async () => {
    api.getAiProviderActivity.mockResolvedValueOnce(REBUILD_RUNNING);
    await mount({});
    await typeChatModel('vendor/other-model');
    await save();

    expect(workDialogShown()).toBe(false);
    expect(api.setAiConfig).toHaveBeenCalledTimes(1);
  });

  it('does not look for work in progress when neither the provider nor a model changes', async () => {
    await mount({});
    await save();

    expect(api.getAiProviderActivity).not.toHaveBeenCalled();
    expect(api.setAiConfig).toHaveBeenCalledTimes(1);
  });

  it('still saves, and logs why, when the work in progress cannot be read', async () => {
    api.getAiProviderActivity.mockRejectedValueOnce('queue unavailable');
    await mount({});
    await typeChatModel('vendor/other-model');
    await save();

    expect(addLog).toHaveBeenCalledWith('error', 'ai', 'settings:aiWork.checkFailed: queue unavailable');
    expect(api.setAiConfig).toHaveBeenCalledTimes(1);
  });
});
