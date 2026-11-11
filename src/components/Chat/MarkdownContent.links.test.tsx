// A chat answer can echo text from received mail, so a link in it opens only
// after the user has seen where it really goes.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { openMock } = vi.hoisted(() => ({ openMock: vi.fn() }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: openMock }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { MarkdownContent } from './MarkdownContent';

const URL = 'https://example.com/a?q=1';

describe('MarkdownContent external links', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    openMock.mockReset();
    openMock.mockResolvedValue(undefined);
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => root.render(<MarkdownContent content={`[read this](${URL})`} sources={[]} accountId="acc-1" />));
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  function clickLink(): MouseEvent {
    const event = new MouseEvent('click', { bubbles: true, cancelable: true });
    act(() => {
      container.querySelector('a')?.dispatchEvent(event);
    });
    return event;
  }

  it('shows the real address and opens nothing on the first click', () => {
    const event = clickLink();

    expect(event.defaultPrevented).toBe(true);
    expect(container.textContent).toContain(URL);
    expect(openMock).not.toHaveBeenCalled();
  });

  it('opens the address once the user confirms', async () => {
    clickLink();

    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="external-link-open"]')?.click();
    });

    expect(openMock).toHaveBeenCalledWith(URL);
  });
});
