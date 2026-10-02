// The From account's signature in the inline reply/forward composer: below a
// reply, above a forwarded message, and kept (once) when an AI draft replaces
// the text.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => <div data-testid="body">{value}</div>,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  autocompleteRecipients: vi.fn(async () => []),
  getAccountSignature: vi.fn(async (accountId: string) => ({
    accountId,
    html: '<p>Ana Lopez</p>',
    useForNew: false,
    useForReplies: true,
    updatedAt: 1,
  })),
}));

import { useSignatureStore } from '@/stores/signatureStore';
import type { Account, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  senderEmail: 'bea@example.com',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useSignatureStore.setState({ byAccount: {} });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(initialBody: string, mode: 'reply' | 'forward') {
  await act(async () => {
    root.render(
      <ReplyCompose
        email={email}
        threadEmails={[email]}
        accounts={[account]}
        defaultAccountId="a1"
        onSend={async () => {}}
        onCancel={() => {}}
        initialBody={initialBody}
        mode={mode}
      />,
    );
  });
}

const body = () => container.querySelector('[data-testid="body"]')?.textContent ?? '';
const SIGNATURE = '<div data-emailops-signature=""><p>Ana Lopez</p></div>';

describe('ReplyCompose signature', () => {
  it('puts the signature below the reply', async () => {
    await render('', 'reply');
    expect(body()).toBe(`<p></p><p></p>${SIGNATURE}`);
  });

  it('puts the signature above the forwarded message', async () => {
    await render('\n\n---------- Forwarded message ----------\nOriginal text', 'forward');
    const html = body();
    expect(html.startsWith(`<p></p>${SIGNATURE}`)).toBe(true);
    expect(html.indexOf('Ana Lopez')).toBeLessThan(html.indexOf('Forwarded message'));
  });

  it('keeps one signature below an AI draft that arrives later', async () => {
    await render('', 'reply');
    await render('Dear Bea,\n\nThanks for the update.', 'reply');
    const html = body();
    expect(html.match(/data-emailops-signature/g)).toHaveLength(1);
    expect(html.indexOf('Dear Bea,')).toBeLessThan(html.indexOf('Ana Lopez'));
  });
});
