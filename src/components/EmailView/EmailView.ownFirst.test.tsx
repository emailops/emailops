// The own-message-first conversation view (Settings → Appearance →
// Conversation view): the user's latest message on top, the rest oldest
// first, every message open. Presentation only: Reply, Reply all and Forward
// quote and attach exactly what they do in the default view.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  listDrafts: vi.fn(async () => []),
  currentPlatform: () => 'macos',
  getThreadReplyTo: vi.fn(async () => ({})),
  getThreadNewContent: vi.fn(),
  getEmailAttachmentMetas: vi.fn(),
  fetchEmailAttachmentBytes: vi.fn(async () => 'aGVsbG8='),
  generateDraft: vi.fn(async () => 'request-1'),
}));
vi.mock('./ThreadEmailItem', () => ({
  ThreadEmailItem: ({
    email,
    isExpanded,
    isLast,
    isFocused,
    isSearchMatch,
  }: {
    email: Email;
    isExpanded: boolean;
    isLast: boolean;
    isFocused?: boolean;
    isSearchMatch?: boolean;
  }) => (
    <div
      data-testid="thread-email"
      data-id={email.id}
      data-expanded={String(isExpanded)}
      data-collapsible={String(!isLast)}
      data-focused={String(!!isFocused)}
      data-search-match={String(!!isSearchMatch)}
    />
  ),
}));
vi.mock('./ReplyCompose', () => ({
  ReplyCompose: ({
    mode,
    email,
    threadEmails,
    initialBody,
    initialAttachments,
  }: {
    mode: string;
    email: Email;
    threadEmails: Email[];
    initialBody: string;
    initialAttachments?: { filename: string }[];
  }) => (
    <div
      data-testid="reply-compose"
      data-mode={mode}
      data-email={email.id}
      data-thread={threadEmails.map((e) => e.id).join(',')}
      data-body={initialBody}
      data-files={(initialAttachments ?? []).map((a) => a.filename).join(',')}
    />
  ),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import type { ConversationView } from '@/lib/conversationView';
import { useConversationViewStore } from '@/stores/conversationViewStore';
import { useEmailStore } from '@/stores/emailStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Account, Email, EmailAttachmentMeta } from '@/types';
import { EmailView } from './EmailView';

const ME = 'me@example.com';
const account = { id: 'a1', email: ME } as Account;

function received(id: string, timestamp: number, overrides: Partial<Email> = {}): Email {
  return {
    id,
    accountId: 'a1',
    threadId: 'thread-a',
    messageId: id,
    subject: 'Venue options',
    sender: 'Bea',
    senderEmail: 'bea@example.com',
    recipients: [ME],
    cc: [],
    body: '',
    snippet: '',
    timestamp,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
    isStarred: false,
    ...overrides,
  };
}

function sent(id: string, timestamp: number, overrides: Partial<Email> = {}): Email {
  return received(id, timestamp, {
    sender: 'Me',
    senderEmail: ME,
    recipients: ['bea@example.com'],
    mailbox: 'sent',
    isSent: true,
    ...overrides,
  });
}

// Oldest first, as the store hands a thread to the reader.
const conversation = [
  received('r1', 1_000),
  sent('s1', 2_000),
  received('r2', 3_000),
  sent('s2', 4_000),
  received('r3', 5_000),
];

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getEmailAttachmentMetas).mockResolvedValue([]);
  vi.mocked(api.getThreadNewContent).mockResolvedValue([]);
  useEmailStore.setState({ snoozes: new Map(), pendingChatDraft: null, searchQuery: '', focusEmailId: null });
  useShortcutStore.setState({ paneCommand: null, listEmails: [] });
  useConversationViewStore.setState({ view: 'own-first' });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function show(thread: Email[]) {
  await act(async () => {
    root.render(
      <EmailView
        threadEmails={thread}
        isLoading={false}
        onClose={() => {}}
        accounts={[account]}
        activeAccountId="a1"
      />,
    );
  });
}

const shown = () =>
  [...container.querySelectorAll<HTMLElement>('[data-testid="thread-email"]')].map((el) => el.dataset.id);
const items = () => [...container.querySelectorAll<HTMLElement>('[data-testid="thread-email"]')];
const messageList = () => container.querySelector<HTMLElement>('[data-testid="conversation-messages"]');

describe('the own-message-first view', () => {
  it('shows the user’s latest message first, then the rest oldest first', async () => {
    await show(conversation);
    expect(shown()).toEqual(['s2', 'r1', 's1', 'r2', 'r3']);
    expect(container.textContent).toContain('Your latest message');
    expect(container.textContent).toContain('The rest of the conversation, oldest first');
  });

  it('opens every message, none of them collapsible', async () => {
    await show(conversation);
    expect(items().every((el) => el.dataset.expanded === 'true')).toBe(true);
    expect(items().every((el) => el.dataset.collapsible === 'false')).toBe(true);
    expect(container.textContent).not.toMatch(/Show \d+ more message/);
  });

  it('says so, and shows every message oldest first, when the user never wrote in the conversation', async () => {
    await show([received('r1', 1_000), received('r2', 2_000), received('r3', 3_000), received('r4', 4_000)]);
    expect(shown()).toEqual(['r1', 'r2', 'r3', 'r4']);
    expect(items().every((el) => el.dataset.expanded === 'true')).toBe(true);
    expect(container.textContent).toContain('You have not written in this conversation');
    expect(container.textContent).not.toContain('Your latest message');
  });

  it('shows a message stored twice once', async () => {
    await show([received('r1', 1_000), sent('s1', 2_000), received('r1', 1_000), received('r2', 3_000)]);
    expect(shown()).toEqual(['s1', 'r1', 'r2']);
  });

  it('shows only the conversation’s own account', async () => {
    await show([received('r1', 1_000), sent('x1', 2_000, { accountId: 'a2' }), received('r2', 3_000)]);
    expect(shown()).toEqual(['r1', 'r2']);
  });

  it('starts another conversation from the top', async () => {
    await show(conversation);
    const list = messageList();
    if (!list) throw new Error('no message list');
    list.scrollTop = 400;

    await show([received('b1', 1_000, { threadId: 'thread-b' }), sent('b2', 2_000, { threadId: 'thread-b' })]);

    expect(shown()).toEqual(['b2', 'b1']);
    expect(messageList()?.scrollTop).toBe(0);
  });

  it('still brings a message opened from a link into view, as the default view does', async () => {
    useEmailStore.setState({ focusEmailId: 'r2' });
    await show(conversation);
    expect(
      items()
        .filter((el) => el.dataset.focused === 'true')
        .map((el) => el.dataset.id),
    ).toEqual(['r2']);
  });

  it('walks in-thread search matches in the order they are shown', async () => {
    const thread = [
      received('r1', 1_000, { snippet: 'the venue deposit' }),
      sent('s1', 2_000, { snippet: 'paid the venue deposit' }),
    ];
    await show(thread);
    const searchButton = [...container.querySelectorAll('button')].find((b) =>
      b.getAttribute('title')?.startsWith('Search in thread'),
    );
    await act(async () => searchButton?.click());
    const input = container.querySelector<HTMLInputElement>('input[placeholder="Search in this thread…"]');
    if (!input) throw new Error('no search input');
    await act(async () => {
      const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
      setValue?.call(input, 'deposit');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const active = items().find((el) => el.dataset.searchMatch === 'true');
    expect(active?.dataset.id).toBe('s1');
  });

  it('highlights the first shown message that matches the list search', async () => {
    useEmailStore.setState({ searchQuery: 'deposit' });
    await show([
      received('r1', 1_000, { snippet: 'the venue deposit' }),
      sent('s1', 2_000, { snippet: 'deposit paid' }),
    ]);
    const active = items().find((el) => el.dataset.searchMatch === 'true');
    expect(active?.dataset.id).toBe('s1');
  });
});

describe('the per-message view', () => {
  it('stays the default: the latest message open, the middle ones folded away', async () => {
    useConversationViewStore.setState({ view: 'per-message' });
    await show(conversation);
    expect(shown()).toEqual(['r1', 'r3']);
    expect(container.textContent).toContain('Show 3 more messages');
    expect(items().map((el) => el.dataset.expanded)).toEqual(['false', 'true']);
    expect(container.textContent).not.toContain('Your latest message');
  });
});

describe('Reply, Reply all and Forward in the own-message-first view', () => {
  const metas: Record<string, EmailAttachmentMeta[]> = {
    r1: [{ id: 'm1', filename: 'floor-plan.pdf', mimeType: 'application/pdf', fileSize: 10 } as EmailAttachmentMeta],
    r3: [{ id: 'm3', filename: 'quote.pdf', mimeType: 'application/pdf', fileSize: 10 } as EmailAttachmentMeta],
  };

  async function composerIn(view: ConversationView, command: 'reply' | 'replyAll' | 'forward') {
    act(() => root.unmount());
    root = createRoot(container);
    useConversationViewStore.setState({ view });
    useShortcutStore.setState({ paneCommand: null });
    vi.mocked(api.getEmailAttachmentMetas).mockImplementation(async (_account, emailId) => metas[emailId] ?? []);
    vi.mocked(api.getThreadNewContent).mockResolvedValue(
      conversation.map((e) => ({ emailId: e.id, text: `Text of ${e.id}` })),
    );
    await show(conversation);
    await act(async () => {
      useShortcutStore.getState().requestPaneCommand(command);
    });
    const composer = container.querySelector<HTMLElement>('[data-testid="reply-compose"]');
    if (!composer) throw new Error('no composer');
    return { ...composer.dataset };
  }

  it.each(['reply', 'replyAll', 'forward'] as const)(
    '%s quotes and attaches what the default view does',
    async (command) => {
      const perMessage = await composerIn('per-message', command);
      const ownFirst = await composerIn('own-first', command);
      expect(ownFirst).toEqual(perMessage);
    },
  );

  it('answers the conversation’s latest message, not the own message shown on top', async () => {
    const composer = await composerIn('own-first', 'reply');
    expect(composer.email).toBe('r3');
    expect(composer.thread).toBe('r1,s1,r2,s2,r3');
  });

  it('forwards the conversation oldest first, with every message’s files', async () => {
    const composer = await composerIn('own-first', 'forward');
    const body = composer.body ?? '';
    const order = conversation.map((e) => body.indexOf(`Text of ${e.id}`));
    expect(order.every((at) => at >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
    expect(composer.files).toBe('floor-plan.pdf,quote.pdf');
  });
});
