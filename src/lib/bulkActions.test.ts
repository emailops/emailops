// Which bulk actions the toolbar offers for a selection of list rows.

import { describe, expect, it } from 'vitest';
import type { Folder } from '@/lib/api';
import type { Account, Email } from '@/types';
import { bulkAvailability, bulkMoveTargets } from './bulkActions';

const row = (id: string, extra: Partial<Email> = {}) =>
  ({
    id,
    accountId: 'imap-1',
    threadId: `t-${id}`,
    mailbox: 'inbox',
    isRead: true,
    isStarred: false,
    ...extra,
  }) as Email;

const account = (id: string, provider: string) => ({ id, provider }) as Account;
const folder = (serverPath: string) => ({ serverPath, displayName: serverPath, delimiter: '/' }) as Folder;

describe('bulkAvailability', () => {
  it('offers archive when any selected row is in the inbox', () => {
    expect(bulkAvailability([row('a', { mailbox: 'archive' }), row('b')]).canArchive).toBe(true);
    expect(bulkAvailability([row('a', { mailbox: 'archive' }), row('b', { mailbox: 'sent' })]).canArchive).toBe(false);
  });

  it('offers snooze when any selected row is in the inbox', () => {
    expect(bulkAvailability([row('a', { mailbox: 'archive' }), row('b')]).canSnooze).toBe(true);
    expect(bulkAvailability([row('a', { mailbox: 'sent' })]).canSnooze).toBe(false);
  });

  it('offers mark read / unread and star / unstar by what the selection holds', () => {
    const mixed = bulkAvailability([row('a', { isRead: false, isStarred: true }), row('b')]);
    expect(mixed).toMatchObject({ canMarkRead: true, canMarkUnread: true, canStar: true, canUnstar: true });

    const allRead = bulkAvailability([row('a'), row('b')]);
    expect(allRead).toMatchObject({ canMarkRead: false, canMarkUnread: true, canStar: true, canUnstar: false });
  });
});

describe('bulkMoveTargets', () => {
  const accounts = [account('imap-1', 'imap'), account('gmail-1', 'gmail')];
  const folders = [folder('Projects'), folder('Receipts')];

  it('lists the inbox and every folder except the one all rows are already in', () => {
    const targets = bulkMoveTargets([row('a', { mailbox: 'folder:Projects' })], accounts, folders, 'imap-1');
    expect(targets?.mailboxes).toEqual(['inbox', 'folder:Receipts']);
    expect(targets?.accountId).toBe('imap-1');
  });

  it('leaves out the inbox when every row is already there', () => {
    const targets = bulkMoveTargets([row('a'), row('b')], accounts, folders, 'imap-1');
    expect(targets?.mailboxes).toEqual(['folder:Projects', 'folder:Receipts']);
  });

  it('is unavailable across accounts, for non-IMAP accounts, for unmovable rows or without loaded folders', () => {
    expect(bulkMoveTargets([row('a'), row('b', { accountId: 'other' })], accounts, folders, 'imap-1')).toBeNull();
    expect(bulkMoveTargets([row('a', { accountId: 'gmail-1' })], accounts, folders, 'gmail-1')).toBeNull();
    expect(bulkMoveTargets([row('a', { mailbox: 'sent' })], accounts, folders, 'imap-1')).toBeNull();
    expect(bulkMoveTargets([row('a')], accounts, folders, 'someone-else')).toBeNull();
    expect(bulkMoveTargets([], accounts, folders, 'imap-1')).toBeNull();
  });
});
