// Block sender and unsubscribe state: cached sender facts are patched for
// every message of the sender, and a block that filed mail in Spam takes
// those conversations out of the visible list.

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getSenderStatus: vi.fn(),
  unsubscribeFromSender: vi.fn(),
  blockSender: vi.fn(),
  unblockSender: vi.fn(),
  listBlockedSenders: vi.fn(async () => []),
}));

import * as api from '@/lib/api';
import type { Email, SenderStatus } from '@/types';
import { threadKey, useEmailStore } from './emailStore';
import { blockedThreadKeys, patchSender, statusKey, useSenderStore } from './senderStore';

const status = (address: string, extra: Partial<SenderStatus> = {}): SenderStatus => ({
  address,
  blocked: false,
  unsubscribe: null,
  unsubscribedAt: null,
  ...extra,
});

function row(id: string, sender: string, extra: Partial<Email> = {}): Email {
  return {
    id,
    accountId: 'acc',
    threadId: `t-${id}`,
    senderEmail: sender,
    mailbox: 'inbox',
    timestamp: 1,
    ...extra,
  } as Email;
}

beforeEach(() => {
  vi.mocked(api.getSenderStatus).mockReset();
  vi.mocked(api.blockSender).mockReset();
  vi.mocked(api.unblockSender).mockReset();
  vi.mocked(api.unsubscribeFromSender).mockReset();
  useSenderStore.setState({ statusByEmail: {}, blocked: [], dialog: null });
  useEmailStore.setState({ emails: [], threadEmails: [], selectedEmail: null, totalCount: 0, tabs: [] });
});

describe('patchSender', () => {
  it('patches every cached message of the sender in that account only', () => {
    const byEmail = {
      [statusKey('acc', 'm1')]: status('deals@shop.example'),
      [statusKey('acc', 'm2')]: status('deals@shop.example'),
      [statusKey('acc', 'm3')]: status('friend@example.com'),
      [statusKey('other', 'm4')]: status('deals@shop.example'),
    };
    const out = patchSender(byEmail, 'acc', 'Deals@Shop.example', { blocked: true });
    expect(out[statusKey('acc', 'm1')].blocked).toBe(true);
    expect(out[statusKey('acc', 'm2')].blocked).toBe(true);
    expect(out[statusKey('acc', 'm3')].blocked).toBe(false);
    expect(out[statusKey('other', 'm4')].blocked).toBe(false);
  });
});

describe('blockedThreadKeys', () => {
  it('picks the inbox and archive conversations of the sender', () => {
    const keys = blockedThreadKeys(
      [
        row('a', 'Deals@Shop.example'),
        row('b', 'deals@shop.example', { mailbox: 'archive' }),
        row('c', 'deals@shop.example', { mailbox: 'sent' }),
        row('d', 'friend@example.com'),
        row('e', 'deals@shop.example', { accountId: 'other' }),
      ],
      'acc',
      'deals@shop.example',
    );
    expect([...keys].sort()).toEqual([threadKey('acc', 't-a'), threadKey('acc', 't-b')]);
  });
});

describe('useSenderStore', () => {
  it('blocking marks the sender blocked and drops their filed conversations from the list', async () => {
    useSenderStore.setState({ statusByEmail: { [statusKey('acc', 'a')]: status('deals@shop.example') } });
    useEmailStore.setState({ emails: [row('a', 'deals@shop.example'), row('b', 'friend@example.com')], totalCount: 2 });
    vi.mocked(api.blockSender).mockResolvedValue({ moved: 1, localOnly: 0, failed: 0 });

    const report = await useSenderStore.getState().block('acc', 'deals@shop.example', true);

    expect(report.moved).toBe(1);
    expect(api.blockSender).toHaveBeenCalledWith('acc', 'deals@shop.example', true);
    expect(useSenderStore.getState().statusByEmail[statusKey('acc', 'a')].blocked).toBe(true);
    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['b']);
    expect(api.listBlockedSenders).toHaveBeenCalled();
  });

  it('blocking without moving keeps the list as it is', async () => {
    useEmailStore.setState({ emails: [row('a', 'deals@shop.example')], totalCount: 1 });
    vi.mocked(api.blockSender).mockResolvedValue({ moved: 0, localOnly: 0, failed: 0 });

    await useSenderStore.getState().block('acc', 'deals@shop.example', false);

    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['a']);
  });

  it('a failed block rejects and changes nothing', async () => {
    useSenderStore.setState({ statusByEmail: { [statusKey('acc', 'a')]: status('deals@shop.example') } });
    vi.mocked(api.blockSender).mockRejectedValue(new Error('db locked'));

    await expect(useSenderStore.getState().block('acc', 'deals@shop.example', true)).rejects.toThrow('db locked');
    expect(useSenderStore.getState().statusByEmail[statusKey('acc', 'a')].blocked).toBe(false);
  });

  it('unblocking clears the blocked state', async () => {
    useSenderStore.setState({
      statusByEmail: { [statusKey('acc', 'a')]: status('deals@shop.example', { blocked: true }) },
    });
    vi.mocked(api.unblockSender).mockResolvedValue({ moved: 2, localOnly: 0, failed: 0 });

    await useSenderStore.getState().unblock('acc', 'deals@shop.example', true);

    expect(api.unblockSender).toHaveBeenCalledWith('acc', 'deals@shop.example', true);
    expect(useSenderStore.getState().statusByEmail[statusKey('acc', 'a')].blocked).toBe(false);
  });

  it('a successful unsubscribe marks every cached message of the sender', async () => {
    useSenderStore.setState({
      statusByEmail: {
        [statusKey('acc', 'a')]: status('deals@shop.example'),
        [statusKey('acc', 'b')]: status('deals@shop.example'),
      },
    });
    vi.mocked(api.unsubscribeFromSender).mockResolvedValue('oneClick');

    expect(await useSenderStore.getState().unsubscribe('acc', 'a')).toBe('oneClick');

    const byEmail = useSenderStore.getState().statusByEmail;
    expect(byEmail[statusKey('acc', 'a')].unsubscribedAt).not.toBeNull();
    expect(byEmail[statusKey('acc', 'b')].unsubscribedAt).not.toBeNull();
  });

  it('a failed status load is logged, not thrown', async () => {
    vi.mocked(api.getSenderStatus).mockRejectedValue(new Error('gone'));
    await expect(useSenderStore.getState().loadStatus('acc', 'a')).resolves.toBeUndefined();
    expect(useSenderStore.getState().statusByEmail).toEqual({});
  });
});
