// Which account a new message is sent from, and the order Compose lists the
// senders in. Pure; the stored choice lives in `src/stores/defaultSenderStore.ts`.

import type { Account, AccountSentCount } from '@/types';

/** SQLite preference holding the account new blank messages start from. Empty
 *  or absent: the account of the selected view. */
export const DEFAULT_SENDER_PREF = 'compose.default_from_account';

export function parseDefaultSender(raw: string | null): string | null {
  return raw ? raw : null;
}

/**
 * The account a new blank message starts from: the chosen default while it is
 * an enabled account, otherwise the selected view's account (the first enabled
 * one under "All accounts"). Replies, drafts and links bring their own account
 * and never come through here.
 */
export function newMessageAccountId(
  accounts: Account[],
  defaultSenderId: string | null,
  viewAccountId: string | null,
): string | null {
  const chosen = accounts.find((a) => a.id === defaultSenderId && a.enabled);
  return chosen ? chosen.id : viewAccountId;
}

/** The enabled accounts, the one with the most sent mail first; accounts with
 *  the same count (or none) keep the order they are given in. */
export function orderSendersByUsage(accounts: Account[], counts: AccountSentCount[]): Account[] {
  const sentBy = new Map(counts.map((c) => [c.accountId, c.sentCount]));
  return accounts.filter((a) => a.enabled).sort((a, b) => (sentBy.get(b.id) ?? 0) - (sentBy.get(a.id) ?? 0));
}
