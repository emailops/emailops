// Before the AI provider or a model changes, the dialog lists the background
// work the change cuts across and lets the user stop it, wait for it, or
// keep things as they are. Nothing is applied until that work is gone.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AiProviderActivity, AiWorkItem, AiWorkKind } from '@/types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
  }),
}));

const api = vi.hoisted(() => ({
  getAiProviderActivity: vi.fn(),
  cancelAiProviderWork: vi.fn(() => Promise.resolve(1)),
}));
vi.mock('@/lib/api', () => api);

import { useLogStore } from '@/stores/logStore';
import { AiWorkInProgressDialog } from './AiWorkInProgressDialog';

const item = (kind: AiWorkKind, over: Partial<AiWorkItem> = {}): AiWorkItem => ({
  kind,
  running: false,
  stopping: false,
  progress: null,
  ...over,
});
const activity = (items: AiWorkItem[], provider = 'ollama'): AiProviderActivity => ({ provider, items });

const PROVIDER_CHANGE = { provider: true, model: true, embeddingModel: true };
const CHAT_MODEL_CHANGE = { provider: false, model: true, embeddingModel: false };

describe('AiWorkInProgressDialog', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onProceed = vi.fn();
  const onCancel = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    vi.clearAllMocks();
    useLogStore.setState({ entries: [] });
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  async function mount(change: typeof PROVIDER_CHANGE, initial: AiProviderActivity) {
    await act(async () => {
      root.render(
        <AiWorkInProgressDialog change={change} activity={initial} onProceed={onProceed} onCancel={onCancel} />,
      );
    });
  }

  async function click(label: string) {
    const found = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === label);
    if (!found) throw new Error(`button ${label} not rendered`);
    await act(async () => {
      found.click();
      await vi.advanceTimersByTimeAsync(0);
    });
  }

  async function aSecondPasses() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
  }

  const text = () => container.textContent ?? '';

  it('lists the work the change cuts across, with progress and what is queued', async () => {
    await mount(
      CHAT_MODEL_CHANGE,
      activity([
        item('embeddingsGeneration', { running: true, progress: { current: 3, total: 50 } }),
        item('classification', { running: true, progress: { current: 12, total: 40 } }),
        item('classification'),
        item('lensExtraction'),
      ]),
    );

    const lines = Array.from(container.querySelectorAll('li')).map((li) => li.textContent);
    expect(lines).toEqual([
      'settings:aiWork.kinds.classification · 12/40 · settings:aiWork.moreQueued {"n":1}',
      'settings:aiWork.kinds.lensExtraction · settings:aiWork.queued {"n":1}',
    ]);
    // A chat-model change does not touch the Embeddings being generated.
    expect(text()).not.toContain('settings:aiWork.kinds.embeddingsGeneration');
  });

  it('cancels without touching the work or applying anything', async () => {
    await mount(PROVIDER_CHANGE, activity([item('classification', { running: true })]));
    await click('common:actions.cancel');

    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onProceed).not.toHaveBeenCalled();
    expect(api.cancelAiProviderWork).not.toHaveBeenCalled();
    expect(api.getAiProviderActivity).not.toHaveBeenCalled();
  });

  it('stops the affected work and applies the change once it has ended', async () => {
    const running = activity([item('classification', { running: true }), item('embeddingsGeneration')]);
    api.getAiProviderActivity
      // The running task is still finishing its email…
      .mockResolvedValueOnce(activity([item('classification', { running: true, stopping: true })]))
      // …and a second later only work the change does not touch is left.
      .mockResolvedValue(activity([item('embeddingsGeneration', { running: true })]));
    await mount(CHAT_MODEL_CHANGE, running);

    await click('settings:aiWork.stop');
    expect(api.cancelAiProviderWork).toHaveBeenCalledWith([
      'classification',
      'memoryExtraction',
      'taskExtraction',
      'lensExtraction',
      'agentRules',
    ]);
    expect(onProceed).not.toHaveBeenCalled();
    expect(text()).toContain('settings:aiWork.stopping');

    await aSecondPasses();
    expect(onProceed).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it('waits without stopping anything, and applies the change when the work is done', async () => {
    const running = activity([item('embeddingsRebuild', { running: true, progress: { current: 1, total: 500 } })]);
    api.getAiProviderActivity
      .mockResolvedValueOnce(
        activity([item('embeddingsRebuild', { running: true, progress: { current: 9, total: 500 } })]),
      )
      .mockResolvedValue(activity([]));
    await mount(PROVIDER_CHANGE, running);

    await click('settings:aiWork.wait');
    expect(text()).toContain('settings:aiWork.waiting');
    expect(text()).toContain('settings:aiWork.kinds.embeddingsRebuild · 9/500');
    expect(onProceed).not.toHaveBeenCalled();

    await aSecondPasses();
    expect(onProceed).toHaveBeenCalledTimes(1);
    expect(api.cancelAiProviderWork).not.toHaveBeenCalled();
  });

  it('lets the user give up waiting: nothing is applied', async () => {
    api.getAiProviderActivity.mockResolvedValue(activity([item('classification', { running: true })]));
    await mount(PROVIDER_CHANGE, activity([item('classification', { running: true })]));

    await click('settings:aiWork.wait');
    await click('common:actions.cancel');
    expect(onCancel).toHaveBeenCalledTimes(1);

    // The parent closes the dialog; the work ending later applies nothing.
    act(() => root.render(null));
    api.getAiProviderActivity.mockResolvedValue(activity([]));
    await aSecondPasses();
    expect(onProceed).not.toHaveBeenCalled();
  });

  it('says what stopping and waiting mean while Embeddings are being sent to OpenRouter', async () => {
    await mount(PROVIDER_CHANGE, activity([item('embeddingsRebuild', { running: true })], 'openrouter'));
    expect(text()).toContain('settings:aiWork.openRouter');
  });

  it('does not mention OpenRouter for local Embeddings or for other work', async () => {
    await mount(PROVIDER_CHANGE, activity([item('embeddingsRebuild', { running: true })], 'ollama'));
    expect(text()).not.toContain('settings:aiWork.openRouter');
    await mount(PROVIDER_CHANGE, activity([item('classification', { running: true })], 'openrouter'));
    expect(text()).not.toContain('settings:aiWork.openRouter');
  });

  it('shows and logs a failure to stop the work, and applies nothing', async () => {
    api.cancelAiProviderWork.mockRejectedValueOnce('queue unavailable');
    await mount(PROVIDER_CHANGE, activity([item('classification', { running: true })]));

    await click('settings:aiWork.stop');

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('queue unavailable');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error' && e.source === 'ai')).toBe(true);
    expect(onProceed).not.toHaveBeenCalled();
    // Back to the three choices.
    expect(text()).toContain('settings:aiWork.wait');
  });
});
