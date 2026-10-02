// The unsubscribe / block / unblock confirmations: what each one tells the
// user before it contacts anyone, and what it calls once confirmed.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => undefined) }));
vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  unsubscribeFromSender: vi.fn(),
  blockSender: vi.fn(),
  unblockSender: vi.fn(),
  listBlockedSenders: vi.fn(async () => []),
  getThread: vi.fn(async () => []),
  getEmailBody: vi.fn(async () => ''),
  markAsRead: vi.fn(async () => {}),
}));

import { open as openExternal } from '@tauri-apps/plugin-shell';
import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useAutoAdvanceStore } from '@/stores/autoAdvanceStore';
import { useEmailStore } from '@/stores/emailStore';
import { statusKey, useSenderStore } from '@/stores/senderStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email, SenderStatus, UnsubscribeOption } from '@/types';
import { SenderDialogs } from './SenderDialogs';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
  useSenderStore.setState({ statusByEmail: {}, blocked: [], dialog: null });
});

function seed(option: UnsubscribeOption, extra: Partial<SenderStatus> = {}) {
  useSenderStore.setState({
    statusByEmail: {
      [statusKey('acc', 'm1')]: {
        address: 'deals@shop.example',
        blocked: false,
        unsubscribe: option,
        unsubscribedAt: null,
        ...extra,
      },
    },
    dialog: { type: 'unsubscribe', accountId: 'acc', emailId: 'm1', senderName: 'Shop Deals' },
  });
}

async function render() {
  await act(async () => {
    root.render(<SenderDialogs />);
  });
}

const byTestId = (id: string) => document.querySelector<HTMLElement>(`[data-testid="${id}"]`);
async function click(el: HTMLElement | null) {
  if (!el) throw new Error('element not rendered');
  await act(async () => {
    el.click();
  });
}

describe('unsubscribe confirmation', () => {
  it('names the host it will contact and that it is the sender, not the provider', async () => {
    seed({ kind: 'oneClick', target: 'lists.shop.example', url: null });
    await render();
    const text = document.body.textContent ?? '';
    expect(text).toContain('Unsubscribe from Shop Deals?');
    expect(text).toContain('lists.shop.example');
    expect(text).toContain('not your mail provider');
    expect(api.unsubscribeFromSender).not.toHaveBeenCalled();
  });

  it('unsubscribes on confirm and then offers to block the sender', async () => {
    seed({ kind: 'oneClick', target: 'lists.shop.example', url: null });
    vi.mocked(api.unsubscribeFromSender).mockResolvedValue('oneClick');
    await render();

    await click(byTestId('unsubscribe-confirm'));

    expect(api.unsubscribeFromSender).toHaveBeenCalledWith('acc', 'm1');
    await click(byTestId('unsubscribe-also-block'));
    expect(useSenderStore.getState().dialog).toEqual({
      type: 'block',
      accountId: 'acc',
      address: 'deals@shop.example',
    });
  });

  it('opens a link option in the browser before recording it', async () => {
    seed({ kind: 'link', target: 'shop.example', url: 'https://shop.example/leave' });
    vi.mocked(api.unsubscribeFromSender).mockResolvedValue('link');
    await render();

    await click(byTestId('unsubscribe-confirm'));

    expect(openExternal).toHaveBeenCalledWith('https://shop.example/leave');
    expect(api.unsubscribeFromSender).toHaveBeenCalledWith('acc', 'm1');
  });

  it('shows the failure in the dialog and keeps it open', async () => {
    seed({ kind: 'mailto', target: 'leave@shop.example', url: null });
    vi.mocked(api.unsubscribeFromSender).mockRejectedValue(new Error('smtp down'));
    await render();

    await click(byTestId('unsubscribe-confirm'));

    expect(document.querySelector('[role="alert"]')?.textContent).toContain('smtp down');
    expect(useSenderStore.getState().dialog?.type).toBe('unsubscribe');
  });
});

describe('block confirmation', () => {
  it('blocks and moves existing mail by default', async () => {
    useSenderStore.setState({ dialog: { type: 'block', accountId: 'acc', address: 'deals@shop.example' } });
    vi.mocked(api.blockSender).mockResolvedValue({ moved: 3, localOnly: 0, failed: 0 });
    await render();

    const checkbox = document.querySelector<HTMLInputElement>('input[type="checkbox"]');
    expect(checkbox?.checked).toBe(true);
    await click(byTestId('sender-block-confirm'));

    expect(api.blockSender).toHaveBeenCalledWith('acc', 'deals@shop.example', true);
    expect(useSenderStore.getState().dialog).toBeNull();
  });

  it('can block without touching existing mail', async () => {
    useSenderStore.setState({ dialog: { type: 'block', accountId: 'acc', address: 'deals@shop.example' } });
    vi.mocked(api.blockSender).mockResolvedValue({ moved: 0, localOnly: 0, failed: 0 });
    await render();

    await click(document.querySelector<HTMLInputElement>('input[type="checkbox"]'));
    await click(byTestId('sender-block-confirm'));

    expect(api.blockSender).toHaveBeenCalledWith('acc', 'deals@shop.example', false);
  });

  it('unblocks, offering to bring their mail back from Spam', async () => {
    useSenderStore.setState({ dialog: { type: 'unblock', accountId: 'acc', address: 'deals@shop.example' } });
    vi.mocked(api.unblockSender).mockResolvedValue({ moved: 1, localOnly: 0, failed: 0 });
    await render();

    expect(document.body.textContent).toContain('back to the inbox');
    await click(byTestId('sender-block-confirm'));

    expect(api.unblockSender).toHaveBeenCalledWith('acc', 'deals@shop.example', true);
  });
});

describe('blocking the sender of the open conversation', () => {
  const row = (id: string, senderEmail: string) =>
    ({ id, accountId: 'acc', threadId: `t-${id}`, mailbox: 'inbox', senderEmail, isRead: true, timestamp: 1 }) as Email;
  const LIST = [row('m1', 'news@other.example'), row('m2', 'deals@shop.example'), row('m3', 'ana@example.com')];

  beforeEach(async () => {
    useAutoAdvanceStore.setState({ mode: 'next' });
    useEmailStore.setState({ emails: LIST, totalCount: 3, tabs: [], activeTabId: null });
    useShortcutStore.setState({ listEmails: LIST });
    vi.mocked(api.blockSender).mockResolvedValue({ moved: 1, localOnly: 0, failed: 0 });
  });

  it('opens the next conversation once their mail has left for Spam', async () => {
    await useEmailStore.getState().selectEmail(LIST[1]);
    useSenderStore.setState({ dialog: { type: 'block', accountId: 'acc', address: 'deals@shop.example' } });
    await render();
    await click(byTestId('sender-block-confirm'));
    expect(useEmailStore.getState().selectedEmail?.id).toBe('m3');
  });

  it('stays on the open conversation when it is from someone else', async () => {
    await useEmailStore.getState().selectEmail(LIST[0]);
    useSenderStore.setState({ dialog: { type: 'block', accountId: 'acc', address: 'deals@shop.example' } });
    await render();
    await click(byTestId('sender-block-confirm'));
    expect(useEmailStore.getState().selectedEmail?.id).toBe('m1');
  });
});
