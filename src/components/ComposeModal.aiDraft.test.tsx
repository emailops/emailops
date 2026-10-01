// A draft outcome that arrives before generate_new_draft returns its request
// id (a fast failure) must still end the "generating" state.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const handlers = new Map<string, (e: { payload: unknown }) => void>();

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    return () => handlers.delete(name);
  }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => <div data-testid="body">{value}</div>,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(),
  getAccountSignature: vi.fn(async (accountId: string) => ({
    accountId,
    html: '',
    useForNew: true,
    useForReplies: true,
    updatedAt: null,
  })),
}));

import * as api from '@/lib/api';
import { useSignatureStore } from '@/stores/signatureStore';
import type { Account } from '@/types';
import { ComposeModal } from './ComposeModal';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
  handlers.clear();
  useSignatureStore.setState({ byAccount: {} });
});

async function typeInto(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

const draftButton = () => document.querySelector<HTMLButtonElement>('button[title="compose:aiDraft.newTitle"]');

describe('ComposeModal AI draft', () => {
  it('stops generating when the failure arrives before the request id', async () => {
    vi.mocked(api.generateNewDraft).mockImplementation(async () => {
      handlers.get('draft-failed')?.({ payload: { requestId: 'r1', emailId: '', error: 'no model' } });
      return 'r1';
    });
    await act(async () => {
      root.render(
        <ComposeModal
          accounts={[account]}
          defaultAccountId="a1"
          defaultToRecipients={['bob@example.com']}
          onClose={() => {}}
        />,
      );
    });
    const subject = document.querySelector<HTMLInputElement>('input[placeholder="compose:subjectPlaceholderLong"]');
    if (!subject) throw new Error('subject not rendered');
    await typeInto(subject, 'Quarterly numbers');

    await act(async () => {
      draftButton()?.click();
    });

    // Not generating any more: the button is usable again.
    expect(draftButton()?.disabled).toBe(false);
  });

  it('inserts the signature and lands the AI draft above it, out of the brief', async () => {
    vi.mocked(api.getAccountSignature).mockResolvedValueOnce({
      accountId: 'a1',
      html: '<p>Ana Lopez</p>',
      useForNew: true,
      useForReplies: true,
      updatedAt: 1,
    });
    vi.mocked(api.generateNewDraft).mockResolvedValue('r2');
    await act(async () => {
      root.render(
        <ComposeModal
          accounts={[account]}
          defaultAccountId="a1"
          defaultToRecipients={['bob@example.com']}
          onClose={() => {}}
        />,
      );
    });
    const body = () => document.querySelector('[data-testid="body"]')?.textContent ?? '';
    expect(body()).toContain('<div data-emailops-signature=""><p>Ana Lopez</p></div>');

    const subject = document.querySelector<HTMLInputElement>('input[placeholder="compose:subjectPlaceholderLong"]');
    if (!subject) throw new Error('subject not rendered');
    await typeInto(subject, 'Quarterly numbers');
    await act(async () => {
      draftButton()?.click();
    });
    // The signature is not part of the brief the model drafts from.
    expect(vi.mocked(api.generateNewDraft).mock.calls[0]?.[3]).toBeNull();

    await act(async () => {
      handlers.get('draft-generated')?.({ payload: { requestId: 'r2', emailId: '', body: 'Dear Bob,', sources: [] } });
    });
    const html = body();
    expect(html.indexOf('Dear Bob,')).toBeGreaterThanOrEqual(0);
    expect(html.indexOf('Dear Bob,')).toBeLessThan(html.indexOf('Ana Lopez'));
    expect(html.match(/data-emailops-signature/g)).toHaveLength(1);
  });

  it('does not save a draft for a composer holding only its signature', async () => {
    vi.mocked(api.getAccountSignature).mockResolvedValueOnce({
      accountId: 'a1',
      html: '<p>Ana Lopez</p>',
      useForNew: true,
      useForReplies: true,
      updatedAt: 1,
    });
    await act(async () => {
      root.render(<ComposeModal accounts={[account]} defaultAccountId="a1" onClose={() => {}} />);
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 900));
    });
    expect(api.saveDraft).not.toHaveBeenCalled();
  });
});
