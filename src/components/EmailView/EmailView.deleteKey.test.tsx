// The Delete key moves the open thread to the Trash, like the trash button —
// and never while the user is typing in a field.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Account, Email } from '@/types';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('./ThreadEmailItem', () => ({ ThreadEmailItem: () => <div data-testid="message" /> }));
vi.mock('./ReplyCompose', () => ({ ReplyCompose: () => <div data-testid="reply" /> }));
vi.mock('./AttachmentLightbox', () => ({ AttachmentLightbox: () => null }));

const deleteEmail = vi.hoisted(() => vi.fn());
vi.mock('@/stores/emailStore', async (orig) => {
  const real = await orig<typeof import('@/stores/emailStore')>();
  real.useEmailStore.setState({ deleteEmail } as never);
  return real;
});
vi.mock('@/lib/api', async (orig) => {
  const real = await orig<typeof import('@/lib/api')>();
  return {
    ...real,
    getPref: vi.fn(async () => null),
    currentPlatform: vi.fn(() => 'linux'),
    getEmailBody: vi.fn(async () => ''),
    getThreadDrafts: vi.fn(async () => []),
    listDrafts: vi.fn(async () => []),
  };
});

import { EmailView } from './EmailView';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const thread = [
  {
    id: 'e1',
    accountId: 'a1',
    threadId: 't1',
    subject: 'Invoice',
    sender: 'Alice',
    senderEmail: 'alice@example.com',
    recipients: ['me@example.com'],
    cc: [],
    timestamp: 1,
    isRead: true,
    body: '',
    snippet: '',
  },
] as unknown as Email[];

let container: HTMLDivElement;
let root: Root;
let onClose: ReturnType<typeof vi.fn<() => void>>;

beforeEach(async () => {
  deleteEmail.mockReset().mockResolvedValue(undefined);
  onClose = vi.fn<() => void>();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => {
    root.render(
      <EmailView threadEmails={thread} isLoading={false} onClose={onClose} accounts={[account]} activeAccountId="a1" />,
    );
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.innerHTML = '';
});

async function press(key: string, target: EventTarget = document.body) {
  await act(async () => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
  });
}

describe('EmailView Delete key', () => {
  it('moves the open thread to the Trash and closes it', async () => {
    await press('Delete');
    expect(deleteEmail).toHaveBeenCalledWith('a1', 'e1');
    expect(onClose).toHaveBeenCalled();
  });

  it('works with Backspace too (the Mac delete key)', async () => {
    await press('Backspace');
    expect(deleteEmail).toHaveBeenCalledTimes(1);
  });

  it('does nothing while typing in a field', async () => {
    const input = document.createElement('input');
    document.body.appendChild(input);
    await press('Delete', input);
    await press('Backspace', input);
    expect(deleteEmail).not.toHaveBeenCalled();
  });
});
