// Onboarding with OpenRouter: the embedding model is optional, chosen from the
// models Settings recommends or typed in, and must pass the same dimension
// probe Settings runs — with the API key just typed — before anything is
// saved. Without that, the model was stored unvalidated and semantic search
// stayed off until a Save in Settings.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

// Stable identity: `t` is a dependency of the step's mount effect.
const translate = (key: string, vars?: { error?: string; model?: string }) =>
  vars?.error ? `${key}: ${vars.error}` : vars?.model ? `${key}: ${vars.model}` : key;
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: translate }),
}));

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(() => Promise.resolve()) }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(() => Promise.resolve(null)) }));

const { addLog } = vi.hoisted(() => ({ addLog: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLog }) => unknown) => selector({ addLog }),
}));

const api = vi.hoisted(() => ({
  getAiConfig: vi.fn(),
  detectAiCapability: vi.fn(() => Promise.resolve({ embeddedAiAvailable: true })),
  listCatalogModels: vi.fn(() => Promise.resolve([])),
  setAiConfig: vi.fn(() => Promise.resolve()),
  validateOpenRouterEmbeddingModel: vi.fn((): Promise<void> => Promise.resolve()),
  testAiProvider: vi.fn(() => Promise.resolve('OK')),
  currentPlatform: vi.fn(() => 'macos'),
}));
vi.mock('@/lib/api', () => api);

import { RECOMMENDED_OPENROUTER_EMBEDDING_MODELS } from '@/components/Settings/AiSettings/openRouterEmbeddingModels';
import { StepAiBackend } from './StepAiBackend';

/** The selector entry that reveals the free-text field. */
const OTHER = '__other__';

const NOTHING = { model: null, embeddingModel: null };

// What `get_ai_config` answers: the saved provider's models are its remembered
// ones, and a saved OpenRouter embedding model has passed the probe.
function savedConfig(over: Record<string, unknown> = {}) {
  const base = {
    provider: 'llamacpp',
    model: '',
    embeddingModel: '',
    monthlyBudgetUsd: 0,
    periodStart: 0,
    hasApiKey: false,
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

describe('StepAiBackend — OpenRouter embedding model', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onNext = vi.fn();

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

  const settle = () => act(async () => new Promise((resolve) => setTimeout(resolve, 0)));

  function button(label: string): HTMLButtonElement {
    const found = Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes(label));
    if (!found) throw new Error(`button ${label} not rendered`);
    return found;
  }

  function field(placeholder: string): HTMLInputElement {
    const input = container.querySelector<HTMLInputElement>(`input[placeholder="${placeholder}"]`);
    if (!input) throw new Error(`field ${placeholder} not rendered`);
    return input;
  }

  const embeddingField = () => field('auth:onboarding.aiBackend.embeddingModelPlaceholder');
  const hasEmbeddingField = () =>
    container.querySelector('input[placeholder="auth:onboarding.aiBackend.embeddingModelPlaceholder"]') !== null;

  function embeddingSelect(): HTMLSelectElement {
    const select = container.querySelector<HTMLSelectElement>(
      'select[aria-label="auth:onboarding.aiBackend.embeddingModel"]',
    );
    if (!select) throw new Error('embedding model selector not rendered');
    return select;
  }

  async function choose(value: string) {
    act(() => {
      embeddingSelect().value = value;
      embeddingSelect().dispatchEvent(new Event('change', { bubbles: true }));
    });
    await settle();
  }

  /** Pick "another model" and type its id. */
  async function typeOtherModel(id: string) {
    await choose(OTHER);
    await type(embeddingField(), id);
  }

  async function type(input: HTMLInputElement, value: string) {
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    act(() => {
      setValue?.call(input, value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await settle();
  }

  /** Mount the step on the OpenRouter card. */
  async function mount(config: Record<string, unknown> = {}) {
    api.getAiConfig.mockResolvedValue(savedConfig(config));
    await act(async () => {
      root.render(<StepAiBackend onBack={() => {}} onNext={onNext} />);
    });
    await settle();
    await act(async () => {
      button('auth:onboarding.aiBackend.openrouterTitle').click();
    });
    await settle();
  }

  async function pressContinue() {
    await act(async () => {
      button('auth:onboarding.aiBackend.continue').click();
    });
    await settle();
  }

  it('starts with no embedding model and says what choosing one sends to OpenRouter', async () => {
    await mount();
    expect(embeddingSelect().value).toBe('');
    expect(hasEmbeddingField()).toBe(false);
    expect(container.textContent).toContain('settings:openRouter.embeddingNotice');
  });

  it('offers none, the recommended models with what mail they suit, and another model', async () => {
    await mount();
    const options = Array.from(embeddingSelect().options).map((o) => [o.value, o.textContent]);

    expect(options).toEqual([
      ['', 'settings:openRouter.embeddingNone'],
      ...RECOMMENDED_OPENROUTER_EMBEDDING_MODELS.map((m) => [
        m.id,
        `${m.id} — ${
          m.languages === 'multilingual'
            ? 'settings:openRouter.embeddingRecommendedMultilingual'
            : 'settings:openRouter.embeddingRecommendedEnglish'
        }`,
      ]),
      [OTHER, 'auth:onboarding.aiBackend.embeddingOther'],
    ]);
    expect(RECOMMENDED_OPENROUTER_EMBEDDING_MODELS.some((m) => m.languages === 'english')).toBe(true);
  });

  it('checks a recommended model with the typed key before saving it', async () => {
    const recommended = RECOMMENDED_OPENROUTER_EMBEDDING_MODELS[0].id;
    await mount();
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await choose(recommended);
    expect(container.textContent).toContain('auth:onboarding.aiBackend.embeddingNeedsCheck');
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith(recommended, 'sk-test');
    expect(api.setAiConfig.mock.calls[0].slice(0, 4)).toEqual([
      'openrouter',
      expect.any(String),
      recommended,
      'sk-test',
    ]);
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('goes back to keyword-only search when none is chosen again: no probe', async () => {
    await mount();
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await choose(RECOMMENDED_OPENROUTER_EMBEDDING_MODELS[0].id);
    await choose('');
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[0].slice(2, 3)).toEqual(['']);
  });

  it('drops a typed model when a recommended one is chosen instead', async () => {
    const recommended = RECOMMENDED_OPENROUTER_EMBEDDING_MODELS[1].id;
    await mount();
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await typeOtherModel('vendor/embed');
    await choose(recommended);
    expect(hasEmbeddingField()).toBe(false);
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith(recommended, 'sk-test');
    expect(api.setAiConfig.mock.calls[0].slice(2, 3)).toEqual([recommended]);
  });

  it('shows a saved recommended model in the selector, without a text field', async () => {
    const recommended = RECOMMENDED_OPENROUTER_EMBEDDING_MODELS[2].id;
    await mount({ provider: 'openrouter', model: 'vendor/model', embeddingModel: recommended, hasApiKey: true });

    expect(embeddingSelect().value).toBe(recommended);
    expect(hasEmbeddingField()).toBe(false);
  });

  it('continues without an embedding model: keyword-only search, no probe', async () => {
    await mount();
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[0].slice(0, 4)).toEqual(['openrouter', expect.any(String), '', 'sk-test']);
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('checks a typed embedding model with the typed key before saving it', async () => {
    await mount();
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await typeOtherModel(' vendor/embed ');
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).toHaveBeenCalledWith('vendor/embed', 'sk-test');
    expect(api.setAiConfig.mock.calls[0].slice(0, 4)).toEqual([
      'openrouter',
      expect.any(String),
      'vendor/embed',
      'sk-test',
    ]);
    expect(api.validateOpenRouterEmbeddingModel.mock.invocationCallOrder[0]).toBeLessThan(
      api.setAiConfig.mock.invocationCallOrder[0],
    );
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('stays on the step and shows why when the model fails the check', async () => {
    api.validateOpenRouterEmbeddingModel.mockRejectedValueOnce('returns 1536-dimension vectors');
    await mount();
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await typeOtherModel('vendor/embed-large');
    await pressContinue();

    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(onNext).not.toHaveBeenCalled();
    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toBe('settings:openRouter.embeddingCheckFailed: returns 1536-dimension vectors');
    // Pinned to the top of the wizard's scroll area, not below the form.
    expect(alert?.className).toContain('sticky');
    expect(addLog).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('1536'));
  });

  it('says zero data retention is what blocks the model, and what the options are', async () => {
    api.validateOpenRouterEmbeddingModel.mockRejectedValueOnce({
      code: 'ai_data_policy',
      params: { model: 'vendor/embed' },
      message: 'The model vendor/embed is not available under the data policy',
    });
    await mount({ zeroDataRetention: true });
    await type(field('auth:onboarding.aiBackend.apiKeyPlaceholder'), 'sk-test');
    await typeOtherModel('vendor/embed');
    await pressContinue();

    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(onNext).not.toHaveBeenCalled();
    expect(container.querySelector('[role="alert"]')?.textContent).toBe(
      'settings:openRouter.embeddingZdrBlocked: vendor/embed',
    );
  });

  it('does not check again a saved model that already passed', async () => {
    await mount({
      provider: 'openrouter',
      model: 'vendor/model',
      embeddingModel: 'vendor/embed',
      hasApiKey: true,
    });
    expect(embeddingSelect().value).toBe(OTHER);
    expect(embeddingField().value).toBe('vendor/embed');
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[0].slice(0, 4)).toEqual(['openrouter', 'vendor/model', 'vendor/embed', null]);
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  it('offers the OpenRouter models remembered while another provider is saved, without a second check', async () => {
    await mount({
      hasApiKey: true,
      openRouterValidatedEmbeddingModel: 'vendor/embed',
      remembered: { openrouter: { model: 'vendor/model', embeddingModel: 'vendor/embed' } },
    });
    expect(embeddingSelect().value).toBe(OTHER);
    expect(embeddingField().value).toBe('vendor/embed');
    await pressContinue();

    expect(api.validateOpenRouterEmbeddingModel).not.toHaveBeenCalled();
    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['openrouter', 'vendor/model', 'vendor/embed']);
  });

  it('does not save the in-app embedding model for Ollama', async () => {
    await mount({
      embeddingModel: 'embed-gguf',
      remembered: { ollama: { model: 'ollama-chat', embeddingModel: null } },
    });
    await act(async () => {
      button('auth:onboarding.aiBackend.ollamaTitle').click();
    });
    await settle();
    await pressContinue();

    expect(api.setAiConfig.mock.calls[0].slice(0, 3)).toEqual(['ollama', 'ollama-chat', null]);
  });
});
