// The log panel's quick AI-backend switcher must follow the same rules as the
// Settings pickers: Embedded is unavailable where the runtime cannot run, the
// switch goes through setAiConfig (provider + model saved together), and a
// failed switch puts the previous backend back on screen.

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

const option = (value: string) =>
  container.querySelector<HTMLButtonElement>(`[data-select="dashboard:log.aiBackend"] [data-option="${value}"]`);
const backend = () => container.querySelector('[data-select="dashboard:log.aiBackend"]')?.getAttribute('data-value');

describe('LogPanel ModelSelector', () => {
  it('disables Embedded where the embedded runtime is unavailable', async () => {
    vi.mocked(api.detectAiCapability).mockResolvedValue({ embeddedAiAvailable: false } as never);
    await mount();
    expect(option('llamacpp')?.disabled).toBe(true);
    expect(option('ollama')?.disabled).toBe(false);
  });

  it('saves provider and model through setAiConfig, keeping the other settings', async () => {
    await mount();
    await act(async () => option('llamacpp')?.click());

    expect(api.setAiConfig).toHaveBeenCalledWith('llamacpp', 'qwen-local', 'embed-small', null, 5, true);
    expect(api.setPref).not.toHaveBeenCalledWith('ai_provider', expect.anything());
  });

  it('rolls back to the previous backend when the switch fails', async () => {
    vi.mocked(api.setAiConfig).mockRejectedValueOnce(new Error('disk full'));
    await mount();
    await act(async () => option('llamacpp')?.click());

    expect(backend()).toBe('ollama');
    expect(container.querySelector('[data-select="dashboard:log.aiModel"]')?.getAttribute('data-value')).toBe(
      'llama-small',
    );
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });

  // The model preference is shared by every provider and OpenRouter has no
  // list to pick from: switching to it here would send a local model id.
  it('does not offer OpenRouter unless it is the saved backend', async () => {
    await mount();
    expect(option('openrouter')?.disabled).toBe(true);
    await act(async () => option('openrouter')?.click());
    expect(api.setAiConfig).not.toHaveBeenCalled();
  });

  it('keeps OpenRouter selectable while it is the saved backend', async () => {
    vi.mocked(api.getAiConfig).mockResolvedValue({ ...config, provider: 'openrouter', model: 'vendor/model' } as never);
    await mount();
    expect(backend()).toBe('openrouter');
    expect(option('openrouter')?.disabled).toBe(false);
  });

  // A switch that changes who computes the Embeddings replaces the email
  // index; only Settings asks before doing that.
  it('does not switch to a backend whose embedding model differs from the one in use', async () => {
    vi.mocked(api.getAiConfig).mockResolvedValue({
      ...config,
      remembered: { ...config.remembered, llamacpp: { model: 'qwen-local', embeddingModel: 'embed-gguf' } },
    } as never);
    await mount();

    expect(option('llamacpp')?.disabled).toBe(true);
    expect(option('llamacpp')?.textContent).toBe('dashboard:log.switchInSettings');
    await act(async () => option('llamacpp')?.click());
    expect(api.setAiConfig).not.toHaveBeenCalled();
  });

  it('does not switch to a backend that has no embedding model remembered', async () => {
    vi.mocked(api.getAiConfig).mockResolvedValue({
      ...config,
      remembered: { ...config.remembered, llamacpp: { model: null, embeddingModel: null } },
    } as never);
    await mount();

    expect(option('llamacpp')?.disabled).toBe(true);
  });

  it('does not leave OpenRouter for a local backend', async () => {
    vi.mocked(api.getAiConfig).mockResolvedValue({
      ...config,
      provider: 'openrouter',
      model: 'vendor/model',
      embeddingModel: 'vendor/embed',
    } as never);
    await mount();

    expect(option('llamacpp')?.disabled).toBe(true);
    expect(option('ollama')?.disabled).toBe(true);
  });

  it('switches to the chat model remembered for the backend', async () => {
    vi.mocked(api.listCatalogModels).mockResolvedValueOnce([
      { id: 'other-local', kind: 'chat', isLocal: true },
      { id: 'qwen-local', kind: 'chat', isLocal: true },
    ] as never);
    await mount();
    await act(async () => option('llamacpp')?.click());

    expect(vi.mocked(api.setAiConfig).mock.calls[0].slice(0, 3)).toEqual(['llamacpp', 'qwen-local', 'embed-small']);
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

  it('asks before switching backend while AI work is in progress', async () => {
    vi.mocked(api.getAiProviderActivity).mockResolvedValue(CLASSIFYING);
    await mount();
    await act(async () => option('llamacpp')?.click());

    expect(container.textContent).toContain('settings:aiWork.title');
    expect(api.setAiConfig).not.toHaveBeenCalled();
    expect(backend()).toBe('ollama');
  });
});
