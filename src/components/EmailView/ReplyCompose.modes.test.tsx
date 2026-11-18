// The panel stays mounted between Reply and Reply All, so its recipients have
// to follow the mode; it must not send a forward before the files it carries
// arrive; and a failed send has to be readable wherever the panel is scrolled.

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
  // A forward's files are stored for its draft as they arrive.
  storeDraftFile: vi.fn(async (_accountId: string, file: { filename: string; mimeType: string }) => ({
    filePath: `attachments/a1/drafts/${file.filename}`,
    filename: file.filename,
    mimeType: file.mimeType,
  })),
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
  recipients: ['me@example.com', 'carol@example.com'],
  cc: ['dan@example.com'],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(props: Partial<Parameters<typeof ReplyCompose>[0]> = {}) {
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
        {...props}
      />,
    );
  });
}

const recipients = () =>
  [...container.querySelectorAll('span.rounded-full')].map((chip) => chip.firstChild?.textContent);

describe('moving between Reply and Reply All', () => {
  it('addresses everyone on Reply All, and only the sender back on Reply', async () => {
    await render({ mode: 'reply' });
    expect(recipients()).toEqual(['alice@example.com']);

    await render({ mode: 'reply-all' });
    expect(recipients()).toEqual(['alice@example.com', 'carol@example.com', 'dan@example.com']);

    await render({ mode: 'reply' });
    expect(recipients()).toEqual(['alice@example.com']);
  });

  it('keeps recipients the user already changed', async () => {
    await render({ mode: 'reply-all' });
    const removeCarol = [...container.querySelectorAll('span.rounded-full')]
      .find((chip) => chip.textContent?.includes('carol@example.com'))
      ?.querySelector('button');
    await act(async () => removeCarol?.click());

    await render({ mode: 'reply' });

    expect(recipients()).toEqual(['alice@example.com', 'dan@example.com']);
  });

  it('does not count as an edit worth a draft', async () => {
    await render({ mode: 'reply' });
    await render({ mode: 'reply-all' });

    await act(async () => root.unmount());
    root = createRoot(container);

    expect(api.saveDraft).not.toHaveBeenCalled();
  });
});

describe('a forward whose files are still loading', () => {
  const send = () => container.querySelector<HTMLButtonElement>('[data-testid="reply-send"]');

  it('cannot be sent yet, and says why', async () => {
    const onSend = vi.fn(async () => {});
    await render({
      mode: 'forward',
      initialBody: 'Forwarded message',
      attachmentsLoading: true,
      onSend,
    });
    const to = container.querySelector<HTMLInputElement>('#to-input');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    await act(async () => {
      setter?.call(to, 'bob@example.com');
      to?.dispatchEvent(new Event('input', { bubbles: true }));
    });

    expect(send()?.disabled).toBe(true);
    expect(send()?.textContent).toContain('compose:loadingFiles');

    await act(async () => {
      send()?.click();
      to?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', metaKey: true, bubbles: true }));
    });
    expect(onSend).not.toHaveBeenCalled();
  });

  it('can be sent once they are in', async () => {
    await render({ mode: 'forward', initialBody: 'Forwarded message', attachmentsLoading: true });
    const file = { filename: 'contract.pdf', mimeType: 'application/pdf', data: 'aGVsbG8=' };
    await render({
      mode: 'forward',
      initialBody: 'Forwarded message',
      attachmentsLoading: false,
      initialAttachments: [file],
    });
    const to = container.querySelector<HTMLInputElement>('#to-input');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    await act(async () => {
      setter?.call(to, 'bob@example.com');
      to?.dispatchEvent(new Event('input', { bubbles: true }));
    });

    expect(send()?.disabled).toBe(false);
    expect(container.textContent).toContain('contract.pdf');
  });
});

describe('a failed send', () => {
  it('shows above the panel content, pinned while it scrolls', async () => {
    await render({
      mode: 'reply',
      initialBody: 'Thanks',
      onSend: async () => {
        throw new Error('Authentication required');
      },
    });

    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="reply-send"]')?.click());

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('Authentication required');
    expect(alert?.parentElement?.firstElementChild).toBe(alert);
    expect(alert?.className).toContain('sticky');
  });
});
