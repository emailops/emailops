// #274: files attached to a reply or a forward are kept with its draft, and a
// forward is kept as a draft too.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value, onChange }: { value: string; onChange: (html: string) => void }) => (
    <textarea data-testid="body" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/components/shared/AttachMenu', () => ({ AttachMenu: () => null }));

const stored = { filePath: 'attachments/a1/drafts/0b1c.pdf', filename: 'plan.pdf', mimeType: 'application/pdf' };
const api = vi.hoisted(() => ({
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(),
  deleteDraft: vi.fn(),
  storeDraftFile: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

import type { Account, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.com', name: 'Me', enabled: true } as Account;
const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  senderEmail: 'alice@example.com',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  api.saveDraft.mockReset().mockImplementation(async (req: { id?: string }) => ({ id: req.id ?? 'draft-new' }));
  api.deleteDraft.mockReset().mockResolvedValue(undefined);
  api.storeDraftFile.mockReset().mockResolvedValue(stored);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  container.remove();
});

function render(props: Partial<Parameters<typeof ReplyCompose>[0]> = {}) {
  act(() => {
    root.render(
      <ReplyCompose
        email={email}
        threadEmails={[email]}
        accounts={[account]}
        defaultAccountId="a1"
        onSend={async () => {}}
        onCancel={() => {}}
        initialBody=""
        mode="reply"
        {...props}
      />,
    );
  });
}

async function attach(name: string, content: string) {
  const input = container.querySelector<HTMLInputElement>('input[type="file"]');
  if (!input) throw new Error('file input not rendered');
  const file = new File([content], name, { type: 'application/pdf' });
  Object.defineProperty(input, 'files', { value: [file], configurable: true });
  await act(async () => {
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  // FileReader and the store resolve on later ticks.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
}

async function typeBody(text: string) {
  const area = container.querySelector<HTMLTextAreaElement>('[data-testid="body"]');
  if (!area) throw new Error('editor not rendered');
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(area, text);
    area.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function leave() {
  await act(async () => root.unmount());
}

describe('ReplyCompose files and drafts', () => {
  it('stores an attached file and saves it with the reply draft', async () => {
    render();
    await attach('plan.pdf', 'quarterly plan');
    expect(api.storeDraftFile).toHaveBeenCalledWith('a1', expect.objectContaining({ filename: 'plan.pdf' }));
    await leave();
    expect(api.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ emailId: 'e1', attachments: [stored] }));
  });

  it('keeps a forward as a new-message draft of the sending account', async () => {
    render({ mode: 'forward' });
    await typeBody('<p>FYI</p>');
    await leave();
    expect(api.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ accountId: 'a1', subject: 'Fwd: Hello' }));
    expect(api.saveDraft.mock.calls[0][0]).not.toHaveProperty('emailId');
  });

  it('sends a reply whose draft holds files through that draft', async () => {
    const onSend = vi.fn(async () => {});
    render({ onSend });
    await attach('plan.pdf', 'quarterly plan');
    await typeBody('<p>See attached</p>');
    const send = [...container.querySelectorAll('button')].find((b) => b.textContent === 'compose:sendReply');
    await act(async () => send?.click());
    expect(onSend).toHaveBeenCalledWith(
      expect.objectContaining({ draftHoldsFiles: true, attachments: [], draftId: 'draft-new' }),
    );
  });
});
