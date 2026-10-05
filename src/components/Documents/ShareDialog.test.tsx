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

vi.mock('@/lib/api', () => ({
  getOrganizationDomain: vi.fn(() => Promise.resolve('acme.example')),
  listContacts: vi.fn(() =>
    Promise.resolve({
      items: [{ email: 'ben@acme.example', name: 'Ben' }],
      total: 1,
      hasMore: false,
    }),
  ),
  autocompleteRecipients: vi.fn(() =>
    Promise.resolve([
      { email: 'carla@acme.example', name: 'Carla', domainMatch: true },
      { email: 'carl@example.net', name: 'Carl', domainMatch: false },
    ]),
  ),
}));

import * as api from '@/lib/api';
import { lastRecipientToken, parseRecipients, ShareDialog, withRecipient } from './ShareDialog';

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

describe('recipient typing helpers', () => {
  it('reads the address being typed after the last separator', () => {
    expect(lastRecipientToken('ana@example.com, be')).toBe('be');
    expect(lastRecipientToken('ana@example.com, ')).toBe('');
    expect(lastRecipientToken('')).toBe('');
  });

  it('completes the address being typed and leaves room for the next', () => {
    expect(withRecipient('ana@example.com, be', 'ben@acme.example')).toBe('ana@example.com, ben@acme.example, ');
    expect(withRecipient('', 'ben@acme.example')).toBe('ben@acme.example, ');
    expect(withRecipient('ben@acme.example, b', 'ben@acme.example')).toBe('ben@acme.example, ');
  });
});

describe('ShareDialog', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onShare = vi.fn(() => Promise.resolve());
  const onClose = vi.fn();

  beforeEach(async () => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.clearAllMocks();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    await act(async () => {
      root.render(
        <ShareDialog
          accountId="acc-1"
          title="Plan"
          exclude={['me@acme.example', 'carl@example.net']}
          fromAddress="me@acme.example"
          onShare={onShare}
          onClose={onClose}
        />,
      );
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

  it('offers colleagues from the same organization before anything is typed', () => {
    const chips = [...document.querySelectorAll('[data-testid^="share-suggestion-"]')].map((e) => e.textContent);
    expect(chips).toHaveLength(1);
    expect(chips[0]).toContain('ben@acme.example');
    expect(chips[0]).toContain('documents:shareDialog.sameOrganization');
  });

  it('while typing, asks for matches ranked by the organization domain and fills one in', async () => {
    vi.useFakeTimers();
    act(() => setValue(document.querySelector('[data-testid="share-recipients"]') as HTMLTextAreaElement, 'car'));
    await act(async () => {
      vi.advanceTimersByTime(200);
    });
    vi.useRealTimers();
    expect(api.autocompleteRecipients).toHaveBeenCalledWith('acc-1', 'car', 'acme.example', 6);
    await act(async () =>
      (document.querySelector('[data-testid="share-suggestion-carla@acme.example"]') as HTMLButtonElement).click(),
    );
    expect(document.querySelector('[data-testid="share-suggestion-carl@example.net"]')).toBeNull();
    expect((document.querySelector('[data-testid="share-recipients"]') as HTMLTextAreaElement).value).toBe(
      'carla@acme.example, ',
    );
  });
});
