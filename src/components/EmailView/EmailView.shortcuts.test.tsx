// Keyboard shortcuts reach the open conversation as pane commands; EmailView
// runs them through the same handlers as its toolbar buttons.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  currentPlatform: () => 'macos',
  getEmailBody: vi.fn(async () => 'Original text'),
  getEmailAttachmentMetas: vi.fn(async () => []),
}));
vi.mock('./ThreadEmailItem', () => ({ ThreadEmailItem: () => null }));
vi.mock('./ReplyCompose', () => ({
  ReplyCompose: ({ mode }: { mode: string }) => <div data-testid="reply-compose" data-mode={mode} />,
}));

import { initI18n } from '@/i18n';
import { useAutoAdvanceStore } from '@/stores/autoAdvanceStore';
import { useEmailStore } from '@/stores/emailStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email } from '@/types';
import { EmailView } from './EmailView';

const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  mailbox: 'inbox',
  senderEmail: 'bea@example.com',
  senderName: 'Bea',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Quarterly plan',
  snippet: '',
  isRead: true,
  isStarred: false,
  timestamp: 1,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;
const onClose = vi.fn();
const actions = {
  archiveThreads: vi.fn(async () => {}),
  deleteThreads: vi.fn(async () => {}),
  setThreadsRead: vi.fn(async () => {}),
  setThreadsStarred: vi.fn(async () => {}),
};

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  onClose.mockClear();
  for (const fn of Object.values(actions)) fn.mockClear();
  useEmailStore.setState({ ...actions, snoozes: new Map() });
  useShortcutStore.setState({ paneCommand: null, listEmails: [] });
  useAutoAdvanceStore.setState({ mode: 'next' });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(
      <EmailView threadEmails={[email]} isLoading={false} onClose={onClose} accounts={[]} activeAccountId="a1" />,
    );
  });
}

async function command(c: Parameters<ReturnType<typeof useShortcutStore.getState>['requestPaneCommand']>[0]) {
  await act(async () => {
    useShortcutStore.getState().requestPaneCommand(c);
  });
}

const reply = () => container.querySelector('[data-testid="reply-compose"]');

describe('EmailView pane commands', () => {
  it('r opens the reply composer, a reply-all, f a forward', async () => {
    await render();
    await command('reply');
    expect(reply()?.getAttribute('data-mode')).toBe('reply');
    await command('replyAll');
    expect(reply()?.getAttribute('data-mode')).toBe('reply-all');
    await command('forward');
    expect(reply()?.getAttribute('data-mode')).toBe('forward');
  });

  it('pressing r twice keeps the composer open (no toggle)', async () => {
    await render();
    await command('reply');
    await command('reply');
    expect(reply()).not.toBeNull();
  });

  it('e archives the conversation and leaves it, like the toolbar button', async () => {
    await render();
    await command('archive');
    expect(actions.archiveThreads).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't1' }]);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('s stars, Shift+U marks unread and goes back, # deletes', async () => {
    await render();
    await command('star');
    expect(actions.setThreadsStarred).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't1' }], true);
    await command('markUnread');
    expect(actions.setThreadsRead).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't1' }], false);
    await command('delete');
    expect(actions.deleteThreads).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't1' }]);
  });

  it('b opens the snooze picker', async () => {
    await render();
    expect(container.querySelector('[data-testid="snooze-options"]')).toBeNull();
    await command('snooze');
    expect(container.querySelector('[data-testid="snooze-options"]')).not.toBeNull();
  });

  it('a command issued before the conversation was shown is not replayed', async () => {
    useShortcutStore.getState().requestPaneCommand('delete');
    await render();
    expect(actions.deleteThreads).not.toHaveBeenCalled();
  });
});

describe('EmailView auto-advance', () => {
  const prev = { ...email, id: 'e0', threadId: 't0' } as Email;
  const next = { ...email, id: 'e2', threadId: 't2' } as Email;
  const selectEmail = vi.fn(async () => {});

  beforeEach(() => {
    selectEmail.mockClear();
    useEmailStore.setState({
      selectEmail,
      setActiveTab: vi.fn(),
      selectedEmail: email,
      activeTabId: null,
      emails: [prev, email, next],
    });
    useShortcutStore.setState({ listEmails: [prev, email, next] });
  });

  it.each([
    ['archive', 'archiveThreads'],
    ['delete', 'deleteThreads'],
  ] as const)('%s opens the next conversation instead of going back to the list', async (cmd, action) => {
    await render();
    await command(cmd);
    expect(actions[action]).toHaveBeenCalled();
    expect(selectEmail).toHaveBeenCalledWith(next, undefined, { markRead: true });
    expect(onClose).not.toHaveBeenCalled();
  });

  it('snoozing from the toolbar opens the next conversation', async () => {
    useEmailStore.setState({ snoozeThreads: vi.fn(async () => {}) });
    await render();
    await command('snooze');
    const first = container.querySelector('[data-testid="snooze-options"] button') as HTMLButtonElement;
    await act(async () => first.click());
    expect(selectEmail).toHaveBeenCalledWith(next, undefined, { markRead: true });
  });

  it('opens the previous one when set to', async () => {
    useAutoAdvanceStore.setState({ mode: 'previous' });
    await render();
    await command('archive');
    expect(selectEmail).toHaveBeenCalledWith(prev, undefined, { markRead: true });
  });

  it('goes back to the list when set to', async () => {
    useAutoAdvanceStore.setState({ mode: 'list' });
    await render();
    await command('archive');
    expect(selectEmail).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('mark as unread always goes back to the list', async () => {
    await render();
    await command('markUnread');
    expect(selectEmail).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
