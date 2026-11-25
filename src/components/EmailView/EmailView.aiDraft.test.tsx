// The reply's AI requests (Regenerate and Polish) never lose the user's text:
// the body stays until the new draft arrives, and after a failure or a
// timeout. A new draft replaces the previous one; a forward's quoted message
// never stays in a reply the AI drafts.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

const handlers = new Map<string, (e: { payload: unknown }) => void>();

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    return () => handlers.delete(name);
  }),
}));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  listDrafts: vi.fn(async () => []),
  currentPlatform: () => 'macos',
  getThreadReplyTo: vi.fn(async () => ({})),
  getThreadNewContent: vi.fn(async () => [{ emailId: 'a1', text: 'Original text' }]),
  getEmailAttachmentMetas: vi.fn(async () => []),
  fetchEmailAttachmentBytes: vi.fn(async () => 'aGVsbG8='),
  generateDraft: vi.fn(async () => 'r1'),
  polishDraft: vi.fn(async () => 'r1'),
}));
vi.mock('./ThreadEmailItem', () => ({ ThreadEmailItem: () => null }));
vi.mock('./ReplyCompose', () => ({
  ReplyCompose: ({
    mode,
    initialBody,
    isLoadingDraft,
    draftError,
    onGenerateDraft,
    onPolishDraft,
  }: {
    mode: string;
    initialBody: string;
    isLoadingDraft?: boolean;
    draftError?: string | null;
    onGenerateDraft?: (instructions: string) => void;
    onPolishDraft?: (instructions: string, draft: string) => void;
  }) => (
    <div
      data-testid="reply-compose"
      data-mode={mode}
      data-body={initialBody}
      data-loading={String(isLoadingDraft ?? false)}
      data-error={draftError ?? ''}
    >
      <button type="button" data-testid="regenerate" onClick={() => onGenerateDraft?.('shorter')} />
      <button type="button" data-testid="polish" onClick={() => onPolishDraft?.('warmer', 'my reply text')} />
    </div>
  ),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { AI_DRAFT_TIMEOUT_MS } from '@/lib/draftRequest';
import { useEmailStore } from '@/stores/emailStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email } from '@/types';
import { EmailView } from './EmailView';

const email = {
  id: 'a1',
  accountId: 'acc',
  threadId: 'thread-a',
  mailbox: 'inbox',
  senderEmail: 'bea@example.com',
  senderName: 'Bea',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Workshop',
  snippet: '',
  isRead: true,
  isStarred: false,
  timestamp: 1,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  useEmailStore.setState({ snoozes: new Map(), pendingChatDraft: null });
  useShortcutStore.setState({ paneCommand: null, listEmails: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  handlers.clear();
});

async function openReply() {
  await act(async () => {
    root.render(
      <EmailView threadEmails={[email]} isLoading={false} onClose={() => {}} accounts={[]} activeAccountId="acc" />,
    );
  });
  await act(async () => {
    useShortcutStore.getState().requestPaneCommand('reply');
  });
}

const composer = () => container.querySelector('[data-testid="reply-compose"]');
const attr = (name: string) => composer()?.getAttribute(name);

async function click(testId: string) {
  await act(async () => container.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`)?.click());
}

async function emit(event: 'draft-generated' | 'draft-failed', payload: Record<string, unknown>) {
  await act(async () => {
    handlers.get(event)?.({ payload: { requestId: 'r1', emailId: '', sources: [], ...payload } });
  });
}

async function generated(body: string) {
  await click('regenerate');
  await emit('draft-generated', { body });
}

describe('the reply AI draft', () => {
  it('keeps the text while a new draft is written, and after it fails', async () => {
    await openReply();
    await generated('First draft');

    await click('regenerate');
    expect(attr('data-loading')).toBe('true');
    expect(attr('data-body')).toBe('First draft');

    await emit('draft-failed', { error: 'model not loaded' });
    expect(attr('data-loading')).toBe('false');
    expect(attr('data-body')).toBe('First draft');
    expect(attr('data-error')).toContain('model not loaded');
  });

  it('replaces the previous draft with the new one', async () => {
    await openReply();
    await generated('First draft');
    await generated('Second draft');
    expect(attr('data-body')).toBe('Second draft');
  });

  it("polishes the reply's own text with the instructions, and puts the result in its place", async () => {
    await openReply();
    await click('polish');
    expect(api.polishDraft).toHaveBeenCalledWith('acc', 'reply', 'my reply text', 'warmer');
    expect(attr('data-mode')).toBe('reply');

    await emit('draft-generated', { body: 'Polished reply' });
    expect(attr('data-body')).toBe('Polished reply');
    expect(attr('data-error')).toBe('');
  });

  it('gives up at the deadline, keeps the text and drops a late draft', async () => {
    await openReply();
    await generated('First draft');
    vi.useFakeTimers();
    await click('polish');
    await act(async () => {
      vi.advanceTimersByTime(AI_DRAFT_TIMEOUT_MS);
    });
    expect(attr('data-loading')).toBe('false');
    expect(attr('data-body')).toBe('First draft');
    expect(attr('data-error')).not.toBe('');

    await emit('draft-generated', { body: 'Late draft' });
    expect(attr('data-body')).toBe('First draft');
  });

  it("never keeps a forward's quoted message in the reply it turns into", async () => {
    await openReply();
    await act(async () => {
      useShortcutStore.getState().requestPaneCommand('forward');
    });
    expect(attr('data-body')).toContain('Original text');

    const aiDraft = [...container.querySelectorAll('button')].find((b) => b.textContent?.includes('AI Draft'));
    await act(async () => aiDraft?.click());

    expect(attr('data-mode')).toBe('reply-all');
    expect(attr('data-body')).toBe('');
  });
});
