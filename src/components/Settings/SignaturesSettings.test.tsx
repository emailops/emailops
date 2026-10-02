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

vi.mock('@/lib/signatureImage', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/signatureImage')>()),
  browserSignatureImageDeps: {
    readDataUrl: async () => 'data:image/png;base64,iVBORw0KGgo=',
    measure: async () => ({ width: 300, height: 80 }),
    resize: async () => 'data:image/png;base64,iVBORw0KGgo=',
  },
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
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

describe('SignaturesSettings — Add image', () => {
  const pick = async (file: File) => {
    const input = container.querySelector<HTMLInputElement>('[data-testid="signature-image-input"]');
    if (!input) throw new Error('file input not rendered');
    Object.defineProperty(input, 'files', { configurable: true, value: [file] });
    await act(async () => {
      input.dispatchEvent(new Event('change', { bubbles: true }));
    });
  };

  it('offers an Add image button that opens a picker limited to PNG, JPEG, GIF and WebP', async () => {
    await render([gmail]);
    expect(button('Add image')).toBeDefined();
    const input = container.querySelector<HTMLInputElement>('[data-testid="signature-image-input"]');
    expect(input?.accept).toBe('image/png,image/jpeg,image/gif,image/webp');
  });

  it('inserts an accepted image into the signature', async () => {
    await render([gmail]);
    await pick(new File(['x'], 'logo.png', { type: 'image/png' }));
    expect(editor()?.value).toBe('<p>Ana</p><p><img src="data:image/png;base64,iVBORw0KGgo=" alt="logo.png"></p>');
    expect(button('Save signature')?.disabled).toBe(false);
  });

  it('refuses an SVG with a visible, logged reason and leaves the signature alone', async () => {
    await render([gmail]);
    await pick(new File(['<svg/>'], 'logo.svg', { type: 'image/svg+xml' }));
    expect(container.textContent).toContain('PNG, JPEG, GIF or WebP');
    expect(editor()?.value).toBe('<p>Ana</p>');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error' && e.message.includes('logo.svg'))).toBe(
      true,
    );
  });
});
