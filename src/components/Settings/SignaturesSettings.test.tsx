// Settings → Signatures: edit an account's signature and its two options,
// save it through the backend (which sanitizes), import Gmail's own.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value, onChange }: { value: string; onChange: (html: string) => void }) => (
    <textarea data-testid="signature-editor" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));
vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getAccountSignature: vi.fn(),
  saveAccountSignature: vi.fn(),
  importProviderSignature: vi.fn(),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useSignatureStore } from '@/stores/signatureStore';
import type { Account, AccountSignature } from '@/types';
import { SignaturesSettings } from './SignaturesSettings';

const gmail = { id: 'g1', email: 'ana@example.com', provider: 'gmail', enabled: true } as Account;
const imap = { id: 'i1', email: 'ana@example.org', provider: 'imap', enabled: true } as Account;

const stored = (accountId: string, html: string): AccountSignature => ({
  accountId,
  html,
  useForNew: true,
  useForReplies: true,
  updatedAt: 1,
});

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useSignatureStore.setState({ byAccount: {} });
  vi.mocked(api.getAccountSignature).mockImplementation(async (id) => stored(id, '<p>Ana</p>'));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function render(accounts: Account[]) {
  await act(async () => {
    root.render(<SignaturesSettings accounts={accounts} />);
  });
}

const editor = () => container.querySelector<HTMLTextAreaElement>('[data-testid="signature-editor"]');
const button = (label: string) => [...container.querySelectorAll('button')].find((b) => b.textContent === label);
const checkbox = (label: string) =>
  [...container.querySelectorAll('label')].find((l) => l.textContent === label)?.querySelector('input');

async function edit(value: string) {
  const el = editor();
  if (!el) throw new Error('editor not rendered');
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('SignaturesSettings', () => {
  it("shows the account's stored signature", async () => {
    await render([gmail]);
    expect(api.getAccountSignature).toHaveBeenCalledWith('g1');
    expect(editor()?.value).toBe('<p>Ana</p>');
  });

  it('saves the edited signature with its options and shows what was stored', async () => {
    vi.mocked(api.saveAccountSignature).mockResolvedValue({
      ...stored('g1', '<p>Ana Lopez</p>'),
      useForReplies: false,
      updatedAt: 2,
    });
    await render([gmail]);
    await edit('<p onclick="x()">Ana Lopez</p>');
    await act(async () => checkbox('Insert in replies and forwards')?.click());
    await act(async () => button('Save signature')?.click());

    expect(api.saveAccountSignature).toHaveBeenCalledWith('g1', {
      html: '<p onclick="x()">Ana Lopez</p>',
      useForNew: true,
      useForReplies: false,
    });
    // The sanitized copy the backend stored replaces the editor content.
    expect(editor()?.value).toBe('<p>Ana Lopez</p>');
    expect(container.textContent).toContain('Signature saved');
    expect(useSignatureStore.getState().byAccount.g1?.html).toBe('<p>Ana Lopez</p>');
  });

  it('shows why a save failed', async () => {
    vi.mocked(api.saveAccountSignature).mockRejectedValue(new Error('too large'));
    await render([gmail]);
    await edit('<p>Big</p>');
    await act(async () => button('Save signature')?.click());
    expect(container.textContent).toContain('too large');
  });

  it("imports Gmail's signature into the editor without saving it", async () => {
    vi.mocked(api.importProviderSignature).mockResolvedValue('<div>Ana from Gmail</div>');
    await render([gmail]);
    await act(async () => button('Import from Gmail')?.click());
    expect(editor()?.value).toBe('<div>Ana from Gmail</div>');
    expect(api.saveAccountSignature).not.toHaveBeenCalled();
    await act(async () => button('Discard changes')?.click());
    expect(editor()?.value).toBe('<p>Ana</p>');
  });

  it('offers the Gmail import only for Gmail accounts', async () => {
    await render([imap]);
    expect(button('Import from Gmail')).toBeUndefined();
  });
});
