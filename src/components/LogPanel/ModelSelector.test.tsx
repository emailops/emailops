// The log panel shows which AI backend is in use and lets the user pick the
// chat model of that backend. Changing the backend itself happens only in AI
// Settings, where the Embeddings warning and the provider checks live.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/Select', () => ({
  Select: ({
    value,
    options,
    onChange,
    ariaLabel,
  }: {
    value: string;
    options: { value: string; label: string; disabled?: boolean }[];
    onChange: (v: string) => void;
    ariaLabel: string;
  }) => (
    <div data-select={ariaLabel} data-value={value}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          data-option={o.value}
          disabled={o.disabled}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  ),
}));
vi.mock('@/lib/api', () => ({
  getAiConfig: vi.fn(),
  listOllamaModels: vi.fn(async () => ['llama-small']),
  listCatalogModels: vi.fn(async () => [{ id: 'qwen-local', kind: 'chat', isLocal: true }]),
  detectAiCapability: vi.fn(),
  setAiConfig: vi.fn(async () => {}),
  setAiModel: vi.fn(async () => {}),
  setPref: vi.fn(async () => {}),
  getAiProviderActivity: vi.fn(),
  cancelAiProviderWork: vi.fn(async () => 1),
}));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { ModelSelector } from './LogPanel';

const config = {
  provider: 'ollama',
  model: 'llama-small',
  embeddingModel: 'embed-small',
  openRouterValidatedEmbeddingModel: null,
  // The in-app provider is remembered with the very embedding model in use:
  // switching to it leaves the Embeddings alone.
  remembered: {
    llamacpp: { model: 'qwen-local', embeddingModel: 'embed-small' },
    ollama: { model: 'llama-small', embeddingModel: 'embed-small' },
    openrouter: { model: null, embeddingModel: null },
  },
  monthlyBudgetUsd: 5,
  periodStart: 0,
  hasApiKey: false,
  thinkingEnabled: true,
  zeroDataRetention: false,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.getAiConfig).mockResolvedValue(config as never);
  vi.mocked(api.detectAiCapability).mockResolvedValue({ embeddedAiAvailable: true } as never);
  vi.mocked(api.getAiProviderActivity).mockResolvedValue({ provider: 'ollama', items: [] });
  useLogStore.setState({ entries: [] });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => {
    root.render(<ModelSelector />);
  });
}

const backendSelect = () => container.querySelector('[data-select="dashboard:log.aiBackend"]');
const backendLabel = () => container.querySelector('[data-testid="ai-backend"]')?.textContent;

describe('LogPanel ModelSelector', () => {
  it('names the backend in use without offering to change it', async () => {
    await mount();

    expect(backendSelect()).toBeNull();
    expect(backendLabel()).toBe('Ollama');
    expect(container.querySelector('[data-select="dashboard:log.aiModel"]')).not.toBeNull();
  });

  it('names OpenRouter and offers no model list for it', async () => {
    vi.mocked(api.getAiConfig).mockResolvedValue({ ...config, provider: 'openrouter', model: 'vendor/model' } as never);
    await mount();

    expect(backendSelect()).toBeNull();
    expect(backendLabel()).toBe('OpenRouter');
    expect(container.querySelector('[data-select="dashboard:log.aiModel"]')).toBeNull();
    expect(api.setAiConfig).not.toHaveBeenCalled();
  });

  // ── Work in progress ───────────────────────────────────────────────────

  const modelOption = (value: string) =>
    container.querySelector<HTMLButtonElement>(`[data-select="dashboard:log.aiModel"] [data-option="${value}"]`);
  const model = () => container.querySelector('[data-select="dashboard:log.aiModel"]')?.getAttribute('data-value');
  const dialogButton = (label: string) =>
    Array.from(container.querySelectorAll('button')).find((b) => b.textContent === label);
  const CLASSIFYING = {
    provider: 'ollama',
    items: [{ kind: 'classification' as const, running: true, stopping: false, progress: null }],
  };

  it('changes the chat model straight away when no AI work uses it', async () => {
    vi.mocked(api.listOllamaModels).mockResolvedValue(['llama-small', 'llama-big']);
    await mount();
    await act(async () => modelOption('llama-big')?.click());

    expect(api.setAiModel).toHaveBeenCalledWith('llama-big');
    expect(container.textContent).not.toContain('settings:aiWork.title');
  });

  it('asks before changing the chat model while AI work uses it, and changes nothing on cancel', async () => {
    vi.mocked(api.listOllamaModels).mockResolvedValue(['llama-small', 'llama-big']);
    vi.mocked(api.getAiProviderActivity).mockResolvedValue(CLASSIFYING);
    await mount();
    await act(async () => modelOption('llama-big')?.click());

    expect(container.textContent).toContain('settings:aiWork.title');
    expect(api.setAiModel).not.toHaveBeenCalled();

    await act(async () => dialogButton('common:actions.cancel')?.click());
    expect(container.textContent).not.toContain('settings:aiWork.title');
    expect(api.setAiModel).not.toHaveBeenCalled();
    expect(model()).toBe('llama-small');
  });

  it('changes the chat model once the work was stopped', async () => {
    vi.mocked(api.listOllamaModels).mockResolvedValue(['llama-small', 'llama-big']);
    vi.mocked(api.getAiProviderActivity)
      .mockResolvedValueOnce(CLASSIFYING)
      .mockResolvedValue({ provider: 'ollama', items: [] });
    await mount();
    await act(async () => modelOption('llama-big')?.click());
    await act(async () => dialogButton('settings:aiWork.stop')?.click());

    expect(api.cancelAiProviderWork).toHaveBeenCalledTimes(1);
    expect(api.setAiModel).toHaveBeenCalledWith('llama-big');
    expect(model()).toBe('llama-big');
  });
});
