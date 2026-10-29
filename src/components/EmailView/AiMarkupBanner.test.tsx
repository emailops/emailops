// Warning above a message that carries text addressed to the AI assistant,
// shown whether or not junk detection is on.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  hasAiControlMarkup: vi.fn(),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { AiMarkupBanner } from './AiMarkupBanner';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function render() {
  await act(async () => {
    root.render(<AiMarkupBanner accountId="acc" emailId="m1" />);
  });
}

describe('AiMarkupBanner', () => {
  it('warns when the message carries text addressed to the AI assistant', async () => {
    vi.mocked(api.hasAiControlMarkup).mockResolvedValue(true);
    await render();

    expect(api.hasAiControlMarkup).toHaveBeenCalledWith('acc', 'm1');
    expect(container.querySelector('[role="alert"]')?.textContent).toContain('AI assistant');
  });

  it('renders nothing for an ordinary message', async () => {
    vi.mocked(api.hasAiControlMarkup).mockResolvedValue(false);
    await render();

    expect(container.textContent).toBe('');
  });

  it('logs a failed check instead of hiding it', async () => {
    vi.mocked(api.hasAiControlMarkup).mockRejectedValue(new Error('db locked'));
    useLogStore.getState().clear();
    await render();

    expect(container.textContent).toBe('');
    const errors = useLogStore.getState().entries.filter((e) => e.level === 'error');
    expect(errors.map((e) => e.message).join()).toContain('db locked');
  });
});
