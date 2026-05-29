import type { Account } from '@/types';

/**
 * Format a queue task label for display.
 *
 * Backend names look like `classify:account:{uuid}` or `chat:turn:{uuid}`;
 * the embedded uuid is noisy and tells the user nothing. We:
 *
 *  1. Find any account_id from the known accounts inside the label.
 *  2. Replace it with its 4-char prefix.
 *  3. Prepend the matching account's email so the user sees which mailbox
 *     the task belongs to at a glance.
 *
 * If no known account_id is found, the label is returned unchanged so we
 * don't lose information for tasks that aren't account-scoped (e.g.
 * `model_download:{model_id}`).
 */
export function formatTaskLabel(rawName: string, accounts: Account[]): string {
  for (const account of accounts) {
    if (rawName.includes(account.id)) {
      const shortened = rawName.split(account.id).join(account.id.slice(0, 4));
      return `${account.email} ${shortened}`;
    }
  }
  return rawName;
}
