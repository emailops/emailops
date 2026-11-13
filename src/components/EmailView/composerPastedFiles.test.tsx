// #260: a file pasted into the body of any composer (Compose, a compose tab,
// Reply or Forward) is attached and stored with the draft, the way a picked or
// dropped file is. The editor's own paste handling is covered in
// `RichTextEditor.files.test.tsx`; here the editor hands over a file directly.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
const pasted = new File(['%PDF-1.7'], 'report.pdf', { type: 'application/pdf' });
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ onAttachFiles }: { onAttachFiles?: (files: File[]) => void }) => (
    <button type="button" data-testid="paste-file" onClick={() => onAttachFiles?.([pasted])} />
  ),
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/components/shared/AttachMenu', () => ({ AttachMenu: () => null }));
const api = vi.hoisted(() => ({
  storeDraftFile: vi.fn(),
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'req'),
}));
vi.mock('@/lib/api', () => api);

import { ComposeModal } from '@/components/ComposeModal';
import type { ComposeTab } from '@/stores/emailStore';
import type { Account, Email } from '@/types';
import { ComposeTabView } from './ComposeTabView';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', provider: 'gmail', email: 'me@example.com', name: 'Me', enabled: true } as Account;
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
const tab: ComposeTab = { type: 'compose', id: 't1', accountId: 'a1', toAddresses: [], subject: '', bodyHtml: '' };

const composers = [
  ['Compose', () => <ComposeModal accounts={[account]} defaultAccountId="a1" onClose={() => {}} />],
  ['a compose tab', () => <ComposeTabView tab={tab} accounts={[account]} onClose={() => {}} />],
  ...(['reply', 'forward'] as const).map(
    (mode) =>
      [
        mode,
        () => (
          <ReplyCompose
            email={email}
            threadEmails={[email]}
            accounts={[account]}
            defaultAccountId="a1"
            onSend={async () => {}}
            onCancel={() => {}}
            initialBody=""
            mode={mode}
          />
        ),
      ] as const,
  ),
] as const;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  api.storeDraftFile.mockResolvedValue({
    filePath: 'attachments/a1/drafts/0b1c.pdf',
    filename: 'report.pdf',
    mimeType: 'application/pdf',
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('a file pasted into the body', () => {
  it.each(composers)('is attached in %s', async (_name, composer) => {
    await act(async () => {
      root.render(composer());
    });
    const body = document.querySelector<HTMLButtonElement>('[data-testid="paste-file"]');
    if (!body) throw new Error('editor not rendered');
    await act(async () => {
      body.click();
    });
    await vi.waitFor(() =>
      expect(api.storeDraftFile).toHaveBeenCalledWith(
        'a1',
        expect.objectContaining({ filename: 'report.pdf', mimeType: 'application/pdf' }),
      ),
    );
    await vi.waitFor(() => expect(document.body.textContent).toContain('report.pdf'));
  });
});
