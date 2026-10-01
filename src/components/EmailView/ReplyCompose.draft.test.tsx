// A reply typed in a thread is kept as a draft: leaving the thread saves what
// was written, opening the thread again brings it back, and sending or
// cancelling removes it. Opening Reply and leaving without typing saves nothing.

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

const api = vi.hoisted(() => ({
  autocompleteRecipients: vi.fn(async () => []),
  getPref: vi.fn(async () => null),
  getFullSignature: vi.fn(async () => ({ text: '', image: null })),
  saveDraft: vi.fn(),
  deleteDraft: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

import type { Account, Draft, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
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

function button(label: string): HTMLButtonElement {
  const b = [...container.querySelectorAll('button')].find((x) => x.textContent === label);
  if (!b) throw new Error(`${label} not rendered`);
  return b;
}

describe('ReplyCompose drafts', () => {
  it('saves nothing when the reply is left untouched', async () => {
    render();
    await leave();
    expect(api.saveDraft).not.toHaveBeenCalled();
  });

  it('saves nothing when only the signature was added to an untouched reply', async () => {
    api.getFullSignature.mockResolvedValueOnce({ text: 'Me\nExample Inc.', image: null });
    render();
    await act(async () => {});
    expect(container.querySelector<HTMLTextAreaElement>('[data-testid="body"]')?.value).toContain('Example Inc.');
    await leave();
    expect(api.saveDraft).not.toHaveBeenCalled();
  });

  it('saves what was typed when the user leaves before the autosave fires', async () => {
    render();
    await typeBody('<p>Hola Alice,</p>');
    await leave();
    expect(api.saveDraft).toHaveBeenCalledTimes(1);
    expect(api.saveDraft).toHaveBeenCalledWith(
      expect.objectContaining({
        accountId: 'a1',
        emailId: 'e1',
        toAddresses: ['alice@example.com'],
        bodyHtml: '<p>Hola Alice,</p>',
        subject: 'Re: Hello',
      }),
    );
  });

  it('reopens a saved draft and keeps editing the same draft', async () => {
    const draft = {
      id: 'draft-1',
      emailId: 'e1',
      accountId: 'a1',
      toAddresses: ['bob@example.com'],
      ccAddresses: [],
      subject: 'Re: Hello',
      body: 'Hola',
      bodyHtml: '<p>Hola</p>',
    } as unknown as Draft;
    render({ restoredDraft: draft });
    expect(container.querySelector<HTMLTextAreaElement>('[data-testid="body"]')?.value).toBe('<p>Hola</p>');
    expect(container.textContent).toContain('bob@example.com');

    await typeBody('<p>Hola Bob, te cuento</p>');
    await leave();
    expect(api.saveDraft).toHaveBeenCalledWith(expect.objectContaining({ id: 'draft-1', emailId: 'e1' }));
  });

  it('tells the thread about each saved draft so reopening Reply continues it', async () => {
    const onDraftSaved = vi.fn();
    render({ onDraftSaved });
    await typeBody('<p>Hola</p>');
    await leave();
    expect(onDraftSaved).toHaveBeenCalledWith(expect.objectContaining({ id: 'draft-new' }));
  });

  it('deletes the saved draft when the reply is cancelled', async () => {
    const onCancel = vi.fn();
    render({ onCancel });
    await typeBody('<p>Hola</p>');
    await act(async () => button('Cancel').click());
    expect(api.deleteDraft).toHaveBeenCalledWith('draft-new', 'a1');
    expect(onCancel).toHaveBeenCalled();
    await leave();
  });

  it('hands the draft id to the send so it is removed with the reply', async () => {
    const onSend = vi.fn(async () => {});
    render({ onSend });
    await typeBody('<p>Hola Alice, te llamo mañana</p>');
    await act(async () => button('Send Reply').click());
    expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ draftId: 'draft-new' }));
    await leave();
    expect(api.saveDraft).toHaveBeenCalledTimes(1);
  });
});
