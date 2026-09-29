// The OpenRouter panel exposes zero data retention as the user's choice.
// "No training on your mail" is not a choice — the backend always sends
// `data_collection: "deny"` — so the panel only states it; the toggle covers
// the stricter, model-costing zero-retention routing.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { OpenRouterPanel } from './OpenRouterPanel';
import type { AiConfigState } from './types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('./UsageSummary', () => ({
  UsageSummary: () => null,
}));

const baseConfig: AiConfigState = {
  provider: 'openrouter',
  model: 'vendor/model',
  embeddingModel: '',
  monthlyBudgetUsd: 0,
  hasApiKey: true,
  thinkingEnabled: false,
  zeroDataRetention: false,
};

describe('OpenRouterPanel', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  function render(config: AiConfigState, setConfig = vi.fn()) {
    act(() => {
      root.render(<OpenRouterPanel config={config} setConfig={setConfig} apiKey="" setApiKey={vi.fn()} />);
    });
    return setConfig;
  }

  function zdrToggle(): HTMLButtonElement {
    const button = container.querySelector<HTMLButtonElement>(
      'button[aria-label="settings:openRouter.zeroDataRetention"]',
    );
    if (!button) throw new Error('zero data retention toggle not rendered');
    return button;
  }

  it('turns zero data retention on', () => {
    const setConfig = render(baseConfig);
    expect(zdrToggle().getAttribute('aria-pressed')).toBe('false');
    act(() => zdrToggle().click());
    expect(setConfig).toHaveBeenCalledWith({ ...baseConfig, zeroDataRetention: true });
  });

  it('shows it on when saved on, and turns it off', () => {
    const setConfig = render({ ...baseConfig, zeroDataRetention: true });
    expect(zdrToggle().getAttribute('aria-pressed')).toBe('true');
    act(() => zdrToggle().click());
    expect(setConfig).toHaveBeenCalledWith({ ...baseConfig, zeroDataRetention: false });
  });

  it('states that providers may never train on mail', () => {
    render(baseConfig);
    expect(container.textContent).toContain('settings:openRouter.noTrainingNotice');
  });
});
