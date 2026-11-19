// A reply draft that reopens with its thread is not saved again unless the
// user changes it, even though the editor re-serialises its HTML (`<div>`
// lines as paragraphs, `<b>` as `<strong>`). Runs the real editor, which is
// what re-serialises.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
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

import type { Account, Draft, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.test', name: 'Me' } as Account;
const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  senderEmail: 'alice@example.test',
  recipients: ['me@example.test'],
  cc: [],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;
const draft = {
  id: 'draft-1',
  emailId: 'e1',
  accountId: 'a1',
  toAddresses: ['alice@example.test'],
  ccAddresses: [],
  bccAddresses: [],
  subject: 'Re: Hello',
  body: 'Line one\nBold line two',
  bodyHtml: '<div>Line one</div><div><b>Bold</b> line two</div>',
} as unknown as Draft;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  container.remove();
  vi.useRealTimers();
  vi.clearAllMocks();
});

describe('ReplyCompose reopening a reply draft', () => {
  it('does not save the draft when it is left unchanged', async () => {
    await act(async () => {
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
          restoredDraft={draft}
        />,
      );
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
    await act(async () => root.unmount());

    expect(api.saveDraft).not.toHaveBeenCalled();
  });
});
