// "Keep model loaded" only means something where a model is loaded on this
// machine (embedded runtime, Ollama). A remote provider has nothing to keep
// in memory, so the field is not offered there.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AiSharedPreferences } from './AiSharedPreferences';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

describe('AiSharedPreferences', () => {
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

  function render(showKeepAlive: boolean) {
    act(() => {
      root.render(
        <AiSharedPreferences
          routingMode="auto"
          onRoutingModeChange={vi.fn()}
          keepAliveMinutes={30}
          onKeepAliveChange={vi.fn()}
          showKeepAlive={showKeepAlive}
          aiMaxEmailCount={1000}
          onMaxEmailCountChange={vi.fn()}
          aiMaxEmailAgeDays={365}
          onMaxEmailAgeDaysChange={vi.fn()}
          nCtx={8192}
          onNCtxChange={vi.fn()}
          showContextWindow={false}
          aiOutputLanguage=""
          onOutputLanguageChange={vi.fn()}
          helpDocsEnabled={false}
          onHelpDocsEnabledChange={vi.fn()}
        />,
      );
    });
  }

  it('offers keep-alive when the model runs on this machine', () => {
    render(true);
    expect(container.textContent).toContain('settings:ai.keepAlive');
  });

  it('hides keep-alive for a remote provider', () => {
    render(false);
    expect(container.textContent).not.toContain('settings:ai.keepAlive');
  });
});
