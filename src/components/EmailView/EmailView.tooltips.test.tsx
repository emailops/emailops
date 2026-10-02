// The reading pane's toolbar names each action's keyboard shortcut in its
// tooltip, read from the shortcut registry, and drops it when shortcuts are off.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  currentPlatform: () => 'macos',
}));
vi.mock('./ThreadEmailItem', () => ({ ThreadEmailItem: () => null }));
vi.mock('./ReplyCompose', () => ({ ReplyCompose: () => null }));

import { initI18n } from '@/i18n';
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

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useEmailStore.setState({ snoozes: new Map() });
  useShortcutStore.setState({ enabled: true, paneCommand: null });
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
      <EmailView threadEmails={[email]} isLoading={false} onClose={() => {}} accounts={[]} activeAccountId="a1" />,
    );
  });
}

const titles = () => Array.from(container.querySelectorAll('header button')).map((b) => b.getAttribute('title'));

describe('EmailView toolbar tooltips', () => {
  it.each([
    'Reply (R)',
    'Reply all (A)',
    'Forward (F)',
    'Archive (E)',
    'Snooze (B)',
    'Mark as unread (Shift+U)',
    'Star this conversation (S)',
    'Delete thread (#)',
  ])('shows %s', async (title) => {
    await render();
    expect(titles()).toContain(title);
  });

  it('drops the keys when keyboard shortcuts are off', async () => {
    useShortcutStore.setState({ enabled: false });
    await render();
    expect(titles()).toContain('Archive');
    expect(titles().some((t) => t?.includes('(E)') || t?.includes('(#)'))).toBe(false);
  });
});
