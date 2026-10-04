// Sharing is the consent to automatic mail: nothing can be shared until the
// user ticks that their changes will be emailed to the addresses they typed.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
  }),
}));

import { parseRecipients, ShareDialog } from './ShareDialog';

function setValue(el: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  setter?.call(el, value);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('parseRecipients', () => {
  it('splits on commas, semicolons and new lines and drops blanks', () => {
    expect(parseRecipients(' ana@example.com, ben@example.org;\ncarl@example.net ,, ')).toEqual([
      'ana@example.com',
      'ben@example.org',
      'carl@example.net',
    ]);
  });
});

describe('ShareDialog', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onShare = vi.fn(() => Promise.resolve());
  const onClose = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.clearAllMocks();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    act(() => {
      root.render(<ShareDialog title="Plan" fromAddress="me@example.com" onShare={onShare} onClose={onClose} />);
    });
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const submit = () => document.querySelector<HTMLButtonElement>('[data-testid="share-submit"]');

  it('stays disabled until there is a recipient and the user consents', () => {
    expect(submit()?.disabled).toBe(true);
    act(() =>
      setValue(document.querySelector('[data-testid="share-recipients"]') as HTMLTextAreaElement, 'ana@example.com'),
    );
    expect(submit()?.disabled).toBe(true);
    act(() => (document.querySelector('[data-testid="share-consent"]') as HTMLInputElement).click());
    expect(submit()?.disabled).toBe(false);
  });

  it('shares with the typed addresses, then closes', async () => {
    act(() =>
      setValue(
        document.querySelector('[data-testid="share-recipients"]') as HTMLTextAreaElement,
        'ana@example.com, ben@example.org',
      ),
    );
    act(() => (document.querySelector('[data-testid="share-consent"]') as HTMLInputElement).click());
    await act(async () => submit()?.click());
    expect(onShare).toHaveBeenCalledWith(['ana@example.com', 'ben@example.org']);
    expect(onClose).toHaveBeenCalled();
  });

  it('keeps the dialog open and shows the error when sharing fails', async () => {
    onShare.mockImplementationOnce(() => Promise.reject(new Error('provider refused')));
    act(() =>
      setValue(document.querySelector('[data-testid="share-recipients"]') as HTMLTextAreaElement, 'ana@example.com'),
    );
    act(() => (document.querySelector('[data-testid="share-consent"]') as HTMLInputElement).click());
    await act(async () => submit()?.click());
    expect(document.querySelector('[data-testid="share-error"]')?.textContent).toContain('provider refused');
    expect(onClose).not.toHaveBeenCalled();
  });
});
