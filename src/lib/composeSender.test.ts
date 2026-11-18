import { describe, expect, it } from 'vitest';
import type { Account, AccountSentCount } from '@/types';
import { newMessageAccountId, orderSendersByUsage, parseDefaultSender } from './composeSender';

function account(id: string, enabled = true): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@mail.example`,
    name: id,
    createdAt: 0,
    sortOrder: 0,
    enabled,
    syncFromTimestamp: null,
    color: 'blue',
  };
}

const sent = (accountId: string, sentCount: number): AccountSentCount => ({ accountId, sentCount });

describe('parseDefaultSender', () => {
  it('reads a stored account id', () => {
    expect(parseDefaultSender('work')).toBe('work');
  });

  it('reads no value or an empty one as following the selected view', () => {
    expect(parseDefaultSender(null)).toBeNull();
    expect(parseDefaultSender('')).toBeNull();
  });
});

describe('newMessageAccountId', () => {
  const accounts = [account('home'), account('work'), account('old', false)];

  it('starts a new message from the chosen default, whatever account the view shows', () => {
    expect(newMessageAccountId(accounts, 'work', 'home')).toBe('work');
  });

  it("follows the view's account when no default is chosen", () => {
    expect(newMessageAccountId(accounts, null, 'home')).toBe('home');
  });

  it("falls back to the view's account when the default is disabled", () => {
    expect(newMessageAccountId(accounts, 'old', 'home')).toBe('home');
  });

  it("falls back to the view's account when the default no longer exists", () => {
    expect(newMessageAccountId(accounts, 'gone', 'home')).toBe('home');
  });

  it('has no account when there is neither a default nor a view account', () => {
    expect(newMessageAccountId([], null, null)).toBeNull();
  });
});

describe('orderSendersByUsage', () => {
  const ids = (list: Account[]) => list.map((a) => a.id);

  it('puts the account with the most sent mail first', () => {
    const accounts = [account('a'), account('b'), account('c')];
    expect(ids(orderSendersByUsage(accounts, [sent('a', 2), sent('b', 9), sent('c', 5)]))).toEqual(['b', 'c', 'a']);
  });

  it('keeps the given order between accounts with the same count, including none', () => {
    const accounts = [account('a'), account('b'), account('c'), account('d')];
    expect(ids(orderSendersByUsage(accounts, [sent('c', 3), sent('b', 3)]))).toEqual(['b', 'c', 'a', 'd']);
  });

  it('lists only enabled accounts', () => {
    const accounts = [account('a'), account('off', false), account('b')];
    expect(ids(orderSendersByUsage(accounts, [sent('off', 50)]))).toEqual(['a', 'b']);
  });

  it('keeps the given order while the counts are unknown', () => {
    const accounts = [account('b'), account('a')];
    expect(ids(orderSendersByUsage(accounts, []))).toEqual(['b', 'a']);
  });
});
