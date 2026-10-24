// Settings → AI with an OpenAI-compatible server: the remote-server
// disclosure, the save wiring (URL, key, embedding source, context size) and
// the OpenRouter controls and warnings when OpenRouter computes the index.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
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
  detectAiCapability: vi.fn(() => Promise.resolve({ embeddedAiAvailable: true as boolean })),
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
  listOpenAiCompatibleModels: vi.fn(() =>
    Promise.resolve([
      { id: 'served-chat', name: 'served-chat' },
      { id: 'served-embed', name: 'served-embed' },
    ]),
  ),
  testAiProvider: vi.fn(() => Promise.resolve('OK')),
  checkOpenAiCompatibleContext: vi.fn(() =>
    Promise.resolve({
      window: 4096,
      source: 'server',
      served: 4096,
      configured: 32768,
      recommended: 16384,
      belowRecommended: true,
    }),
  ),
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

const NOTHING = { model: null, embeddingModel: null };

function savedConfig(server: Record<string, unknown>, over: Record<string, unknown> = {}) {
  return {
    provider: 'openai_compatible',
    model: 'served-chat',
    embeddingModel: '',
    openRouterValidatedEmbeddingModel: null,
    monthlyBudgetUsd: 0,
    periodStart: 0,
    hasApiKey: false,
    thinkingEnabled: false,
    zeroDataRetention: false,
    remembered: {
      llamacpp: NOTHING,
      ollama: NOTHING,
      openrouter: NOTHING,
      openai_compatible: { model: 'served-chat', embeddingModel: '' },
    },
    server: {
      baseUrl: 'http://localhost:1234/v1',
      hasApiKey: false,
      embeddingSource: 'server',
      contextWindow: 0,
      disclosedHost: null,
      ...server,
    },
    ...over,
  };
}

describe('AiSettings — OpenAI-compatible server', () => {
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

  async function mount(server: Record<string, unknown> = {}, over: Record<string, unknown> = {}) {
    api.getAiConfig.mockResolvedValue(savedConfig(server, over));
    await act(async () => {
      root.render(<AiSettings />);
    });
    await settle();
  }

  function button(label: string): HTMLButtonElement {
    const found = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes(label));
    if (!found) throw new Error(`button ${label} not rendered`);
    return found;
  }

  function field(label: string): HTMLInputElement {
    const input = container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`);
    if (!input) throw new Error(`field ${label} not rendered`);
    return input;
  }

  async function type(label: string, value: string) {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    act(() => {
      setValue?.call(field(label), value);
      field(label).dispatchEvent(new Event('input', { bubbles: true }));
    });
    await settle();
  }

  function sourceSelect(): HTMLSelectElement {
    const select = container.querySelector<HTMLSelectElement>('select[aria-label="settings:server.embeddingSource"]');
    if (!select) throw new Error('embedding source selector not rendered');
    return select;
  }

  async function chooseSource(value: string) {
    act(() => {
      sourceSelect().value = value;
      sourceSelect().dispatchEvent(new Event('change', { bubbles: true }));
    });
    await settle();
  }

  async function click(label: string) {
    await act(async () => {
      button(label).click();
    });
    await settle();
  }

  const disclosureShown = () => container.textContent?.includes('settings:server.disclosureTitle') ?? false;

  /** The `server` argument of the last `set_ai_config` call. */
  function savedServer(): Record<string, unknown> {
    const calls = api.setAiConfig.mock.calls as unknown as unknown[][];
    return calls[calls.length - 1][7] as Record<string, unknown>;
  }

  it('saves a server on this machine without asking anything', async () => {
    await mount();
    await click('common:actions.save');

    expect(disclosureShown()).toBe(false);
    expect(api.setAiConfig).toHaveBeenCalledTimes(1);
    expect(savedServer()).toEqual({
      baseUrl: 'http://localhost:1234/v1',
      apiKey: null,
      clearApiKey: false,
      embeddingSource: 'server',
      contextWindow: 0,
      disclosureAcceptedHost: null,
    });
  });

  it('asks before saving a remote server, and saves only once confirmed', async () => {
    await mount();
    await type('settings:server.baseUrl', 'https://llm.example.com/v1');
    await click('common:actions.save');

    expect(disclosureShown()).toBe(true);
    expect(container.textContent).toContain('llm.example.com');
    expect(api.setAiConfig).not.toHaveBeenCalled();

    await click('settings:server.disclosureConfirm');

    expect(api.setAiConfig).toHaveBeenCalledTimes(1);
    expect(savedServer()).toMatchObject({
      baseUrl: 'https://llm.example.com/v1',
      disclosureAcceptedHost: 'llm.example.com',
    });
  });

  it('saves nothing when the disclosure is declined', async () => {
    await mount();
    await type('settings:server.baseUrl', 'https://llm.example.com/v1');
    await click('common:actions.save');
    await click('common:actions.cancel');

    expect(disclosureShown()).toBe(false);
    expect(api.setAiConfig).not.toHaveBeenCalled();
  });

  it('asks again only when the remote host changes', async () => {
    await mount({ baseUrl: 'https://llm.example.com/v1', disclosedHost: 'llm.example.com' });
    await click('common:actions.save');
    expect(disclosureShown()).toBe(false);
    expect(api.setAiConfig).toHaveBeenCalledTimes(1);

    await type('settings:server.baseUrl', 'https://other.example.net/v1');
    await click('common:actions.save');
    expect(disclosureShown()).toBe(true);
  });

  it('sends a new key, or asks for the saved one to be removed', async () => {
    await mount({ hasApiKey: true });
    await type('settings:server.apiKey', 'sk-new');
    await click('common:actions.save');
    expect(savedServer()).toMatchObject({ apiKey: 'sk-new', clearApiKey: false });

    await click('settings:server.removeKey');
    await click('common:actions.save');
    expect(savedServer()).toMatchObject({ apiKey: null, clearApiKey: true });
  });

  it('saves the context size typed in', async () => {
    await mount();
    await type('settings:server.contextWindow', '32768');
    await click('common:actions.save');

    expect(savedServer()).toMatchObject({ contextWindow: 32768 });
  });

  it('offers the in-app model only where the embedded runtime runs', async () => {
    api.detectAiCapability.mockResolvedValueOnce({ embeddedAiAvailable: false });
    await mount();

    expect(Array.from(sourceSelect().options).map((o) => o.value)).toEqual(['server', 'openrouter']);
  });

  it('shows the OpenRouter budget and zero data retention when OpenRouter computes the index', async () => {
    await mount();
    expect(container.textContent).not.toContain('settings:openRouter.zeroDataRetention');

    await chooseSource('openrouter');

    expect(container.textContent).toContain('settings:ai.monthlyBudget');
    expect(container.textContent).toContain('settings:openRouter.zeroDataRetention');
  });

  it('warns that re-indexing sends mail to OpenRouter when it computes the index', async () => {
    await mount(
      {},
      {
        hasApiKey: true,
        embeddingModel: 'served-embed',
        remembered: {
          llamacpp: NOTHING,
          ollama: NOTHING,
          openrouter: NOTHING,
          openai_compatible: { model: 'served-chat', embeddingModel: 'served-embed' },
        },
      },
    );
    await chooseSource('openrouter');
    act(() => {
      const select = container.querySelector<HTMLSelectElement>('select[aria-label="settings:ai.embeddingModel"]');
      if (!select) throw new Error('embedding model selector not rendered');
      select.value = 'vendor/embed';
      select.dispatchEvent(new Event('change', { bubbles: true }));
    });
    await settle();
    await click('common:actions.save');

    expect(container.textContent).toContain('settings:confirmReindex.title');
    expect(container.textContent).toContain('settings:confirmReindex.openRouter');
  });

  it('tests the server typed in, with the key typed in', async () => {
    await mount();
    await type('settings:server.baseUrl', 'http://127.0.0.1:8080/v1');
    await type('settings:server.apiKey', 'sk-typed');
    await click('settings:ai.test');

    expect(api.testAiProvider).toHaveBeenCalledWith(
      'openai_compatible',
      'served-chat',
      'sk-typed',
      'http://127.0.0.1:8080/v1',
    );
  });

  it('reports the context the server loaded when tested, and warns below the recommended size', async () => {
    await mount({ contextWindow: 32768 });
    await click('settings:ai.test');

    expect(api.checkOpenAiCompatibleContext).toHaveBeenCalledWith(
      'http://localhost:1234/v1',
      null,
      'served-chat',
      32768,
    );
    expect(container.textContent).toContain('settings:server.contextCapped');
    expect(container.textContent).toContain('settings:server.contextBelowRecommended');
  });

  it('drops the context report once the window it was measured for changes', async () => {
    await mount({ contextWindow: 32768 });
    await click('settings:ai.test');
    await type('settings:server.contextWindow', '16384');

    expect(container.textContent).not.toContain('settings:server.contextCapped');
  });

  it('checks the context again after a save', async () => {
    await mount();
    await click('common:actions.save');

    expect(api.checkOpenAiCompatibleContext).toHaveBeenCalledWith('http://localhost:1234/v1', null, 'served-chat', 0);
  });
});
