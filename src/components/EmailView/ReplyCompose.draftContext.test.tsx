// An inline reply (or reply-all) is the draft on screen for the chat panel; a
// forward never is: its body carries someone else's message.

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
  currentPlatform: () => 'macos',
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async (req: { id?: string }) => ({ id: req.id ?? 'draft-new' })),
  deleteDraft: vi.fn(async () => {}),
}));
vi.mock('@/lib/api', () => api);

import { useDraftContextStore } from '@/stores/draftContextStore';
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
  subject: 'Q3 report',
  timestamp: 0,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;
const store = () => useDraftContextStore.getState();

beforeEach(() => {
  useDraftContextStore.setState({ active: null, subject: '', hasUserText: false });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(mode: 'reply' | 'reply-all' | 'forward', initialBody = 'The report is attached.') {
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

describe('ReplyCompose as the draft on screen', () => {
  it('offers a reply, with the email it answers and the reply subject', async () => {
    await render('reply');
    expect(store().active?.kind).toBe('reply');
    expect(store().subject).toBe('Re: Q3 report');
    expect(store().hasUserText).toBe(true);
    const snapshot = store().active?.snapshot();
    expect(snapshot?.replyToEmailId).toBe('e1');
    expect(snapshot?.body).toContain('The report is attached.');
    expect(store().active?.accountId()).toBe('a1');
  });

  it('offers a reply-all as such', async () => {
    await render('reply-all');
    expect(store().active?.kind).toBe('replyAll');
  });

  it('never offers a forward', async () => {
    await render('forward');
    expect(store().active).toBeNull();
  });

  it('leaves when it closes', async () => {
    await render('reply');
    act(() => root.render(null));
    expect(store().active).toBeNull();
  });
});
