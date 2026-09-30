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
}));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { ModelSelector } from './LogPanel';

const config = {
  provider: 'ollama',
  model: 'llama-small',
  embeddingModel: 'embed-small',
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

    expect(api.setAiConfig).toHaveBeenCalledWith('llamacpp', 'qwen-local', null, null, 5, true);
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
});
