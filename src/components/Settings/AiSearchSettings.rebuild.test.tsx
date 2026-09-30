// The rebuild button follows the backend's `embedding-progress` events. A
// rebuild the user stopped (before an AI provider or model change) ends with
// `cancelled`: the button must come back, not spin forever.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

type ProgressHandler = (event: { payload: { status: string; message: string } }) => void;
const events = vi.hoisted(() => ({ handler: null as ProgressHandler | null }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((_name: string, handler: ProgressHandler) => {
    events.handler = handler;
    return Promise.resolve(() => {});
  }),
}));
vi.mock('@/lib/api', () => ({
  getEmbeddingsConfig: vi.fn(() => Promise.resolve({ categories: ['primary'] })),
  regenerateEmbeddings: vi.fn(() => Promise.resolve()),
}));

import { AiSearchSettings } from './AiSearchSettings';

describe('AiSearchSettings — rebuild button', () => {
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

  const rebuildButton = () => {
    const found = Array.from(container.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('settings:aiSearch.rebuild'),
    );
    if (!found) throw new Error('rebuild button not rendered');
    return found;
  };

  const emit = (status: string) => act(() => events.handler?.({ payload: { status, message: status } }));

  it.each(['complete', 'error', 'cancelled'])('is available again when the rebuild ends as %s', async (status) => {
    await act(async () => {
      root.render(<AiSearchSettings activeAccountId="acc-1" />);
    });
    await act(async () => rebuildButton().click());
    emit('generating');
    expect(rebuildButton().disabled).toBe(true);

    emit(status);
    expect(rebuildButton().disabled).toBe(false);
    expect(rebuildButton().textContent).toContain('settings:aiSearch.rebuildButton');
  });
});
