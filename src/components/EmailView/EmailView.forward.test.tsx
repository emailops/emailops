// A forward fills in after its panel opens: the quoted messages first, then the
// files. Whatever arrives after the user moved on belongs to nothing on screen,
// and must never reach another thread's composer, where Send would mail it to
// people who were not meant to get it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  listDrafts: vi.fn(async () => []),
  currentPlatform: () => 'macos',
  // No message of the thread names a Reply-To.
  getThreadReplyTo: vi.fn(async () => ({})),
  getThreadNewContent: vi.fn(async () => [{ emailId: 'a1', text: 'Original text' }]),
  getEmailAttachmentMetas: vi.fn(),
  fetchEmailAttachmentBytes: vi.fn(async () => 'aGVsbG8='),
  generateDraft: vi.fn(async () => 'request-1'),
}));
vi.mock('./ThreadEmailItem', () => ({ ThreadEmailItem: () => null }));
vi.mock('./ReplyCompose', () => ({
  ReplyCompose: ({
    mode,
    initialBody,
    initialAttachments,
    attachmentsLoading,
    onCancel,
    onGenerateDraft,
  }: {
    mode: string;
    initialBody: string;
    initialAttachments?: { filename: string }[];
    attachmentsLoading?: boolean;
    onCancel: () => void;
    onGenerateDraft?: (instructions: string) => void;
  }) => (
    <div
      data-testid="reply-compose"
      data-mode={mode}
      data-body={initialBody}
      data-files={(initialAttachments ?? []).map((a) => a.filename).join(',')}
      data-files-loading={String(attachmentsLoading ?? false)}
    >
      <button type="button" data-testid="cancel" onClick={onCancel} />
      <button type="button" data-testid="regenerate" onClick={() => onGenerateDraft?.('shorter')} />
    </div>
  ),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useEmailStore } from '@/stores/emailStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email, EmailAttachmentMeta } from '@/types';
import { EmailView } from './EmailView';

function message(id: string, threadId: string): Email {
  return {
    id,
    accountId: 'a1',
    threadId,
    mailbox: 'inbox',
    senderEmail: 'bea@example.com',
    senderName: 'Bea',
    recipients: ['me@example.com'],
    cc: [],
    subject: `Subject ${threadId}`,
    snippet: '',
    isRead: true,
    isStarred: false,
    timestamp: 1,
  } as unknown as Email;
}

const threadA = message('a1', 'thread-a');
const threadB = message('b1', 'thread-b');
const meta = {
  id: 'm1',
  filename: 'contract.pdf',
  mimeType: 'application/pdf',
  fileSize: 10,
  providerAttachmentId: 'p1',
} as EmailAttachmentMeta;

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getEmailAttachmentMetas).mockResolvedValue([]);
  useEmailStore.setState({ snoozes: new Map(), pendingChatDraft: null });
  useShortcutStore.setState({ paneCommand: null, listEmails: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function show(thread: Email) {
  await act(async () => {
    root.render(
      <EmailView threadEmails={[thread]} isLoading={false} onClose={() => {}} accounts={[]} activeAccountId="a1" />,
    );
  });
}

async function command(c: 'reply' | 'replyAll' | 'forward') {
  await act(async () => {
    useShortcutStore.getState().requestPaneCommand(c);
  });
}

const composer = () => container.querySelector('[data-testid="reply-compose"]');
const files = () => composer()?.getAttribute('data-files');

describe('a forward that fills in late', () => {
  it('keeps its files out of the thread the user moved to', async () => {
    const metas = deferred<EmailAttachmentMeta[]>();
    vi.mocked(api.getEmailAttachmentMetas).mockReturnValueOnce(metas.promise);
    await show(threadA);
    await command('forward');

    await show(threadB);
    await command('reply');
    await act(async () => metas.resolve([meta]));

    expect(composer()?.getAttribute('data-mode')).toBe('reply');
    expect(files()).toBe('');
  });

  it('keeps its files out of a reply opened in the same thread', async () => {
    const metas = deferred<EmailAttachmentMeta[]>();
    vi.mocked(api.getEmailAttachmentMetas).mockReturnValueOnce(metas.promise);
    await show(threadA);
    await command('forward');

    await command('reply');
    await act(async () => metas.resolve([meta]));

    expect(files()).toBe('');
  });

  it('does not open in the thread the user moved to while its text loads', async () => {
    const texts = deferred<{ emailId: string; text: string }[]>();
    vi.mocked(api.getThreadNewContent).mockReturnValueOnce(texts.promise as never);
    await show(threadA);
    await command('forward');

    await show(threadB);
    await act(async () => texts.resolve([{ emailId: 'a1', text: 'Original text' }]));

    expect(composer()).toBeNull();
  });

  it('a cancelled forward leaves no files for the AI draft opened next', async () => {
    vi.mocked(api.getEmailAttachmentMetas).mockResolvedValueOnce([meta]);
    await show(threadA);
    await command('forward');
    expect(files()).toBe('contract.pdf');

    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="cancel"]')?.click());
    const aiDraft = [...container.querySelectorAll('button')].find((b) => b.textContent?.includes('AI Draft'));
    await act(async () => aiDraft?.click());

    expect(composer()?.getAttribute('data-mode')).toBe('reply-all');
    expect(files()).toBe('');
  });
});

describe('pressing Forward again with the forward open', () => {
  it('leaves it as it is, so its files are not attached twice', async () => {
    vi.mocked(api.getEmailAttachmentMetas).mockResolvedValue([meta]);
    await show(threadA);
    await command('forward');
    await command('forward');

    expect(api.getEmailAttachmentMetas).toHaveBeenCalledTimes(1);
    expect(files()).toBe('contract.pdf');
  });
});

describe('the forward panel while its files load', () => {
  it('knows they are on their way until they land', async () => {
    const metas = deferred<EmailAttachmentMeta[]>();
    vi.mocked(api.getEmailAttachmentMetas).mockReturnValueOnce(metas.promise);
    await show(threadA);
    await command('forward');
    expect(composer()?.getAttribute('data-files-loading')).toBe('true');

    await act(async () => metas.resolve([meta]));

    expect(composer()?.getAttribute('data-files-loading')).toBe('false');
    expect(files()).toBe('contract.pdf');
  });

  it('stops waiting when they cannot be loaded', async () => {
    const metas = deferred<EmailAttachmentMeta[]>();
    vi.mocked(api.getEmailAttachmentMetas).mockReturnValueOnce(metas.promise);
    await show(threadA);
    await command('forward');

    await act(async () => metas.reject(new Error('offline')));

    expect(composer()?.getAttribute('data-files-loading')).toBe('false');
  });

  it('is not left waiting by a reply opened meanwhile', async () => {
    const metas = deferred<EmailAttachmentMeta[]>();
    vi.mocked(api.getEmailAttachmentMetas).mockReturnValueOnce(metas.promise);
    await show(threadA);
    await command('forward');

    await command('reply');

    expect(composer()?.getAttribute('data-files-loading')).toBe('false');
  });
});

describe('asking the AI for a new draft in an open composer', () => {
  it('keeps a reply addressed as it was', async () => {
    await show(threadA);
    await command('reply');

    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="regenerate"]')?.click());

    expect(composer()?.getAttribute('data-mode')).toBe('reply');
  });

  it('turns a forward into a reply-all', async () => {
    await show(threadA);
    await command('forward');
    const aiDraft = [...container.querySelectorAll('button')].find((b) => b.textContent?.includes('AI Draft'));

    await act(async () => aiDraft?.click());

    expect(composer()?.getAttribute('data-mode')).toBe('reply-all');
  });
});
