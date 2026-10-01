/**
 * Bulk delete / move for a multi-selection of email rows. Each email gets
 * exactly what the row's ⋮ menu does for it — Delete moves the whole thread
 * to the Trash, Move moves that message — so selecting rows never changes
 * the meaning of an action, only how many emails it applies to.
 *
 * Pure orchestration over injected store functions, so partial failures are
 * unit-tested without IPC (see `bulkActions.test.ts`). Emails are processed
 * one by one: IMAP commands share one connection per account, and a failure
 * on one email must not stop the others.
 */
import type { MailboxView } from '@/lib/api';
import type { Email } from '@/types';

export interface BulkResult {
  /** Emails fully handled. */
  done: number;
  /** Emails that failed, with the error message (first one shown to the user). */
  failed: { emailId: string; error: string }[];
}

export interface BulkDeps {
  getThread: (accountId: string, threadId: string) => Promise<Email[]>;
  deleteEmail: (accountId: string, emailId: string) => Promise<void>;
  moveEmail: (accountId: string, emailId: string, targetMailbox: MailboxView) => Promise<void>;
}

function message(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

/** Move every selected thread to the Trash, deleting each thread only once. */
export async function bulkDelete(emails: Email[], deps: BulkDeps): Promise<BulkResult> {
  const result: BulkResult = { done: 0, failed: [] };
  const seenThreads = new Set<string>();
  for (const email of emails) {
    const key = `${email.accountId}\u0000${email.threadId}`;
    if (seenThreads.has(key)) {
      result.done += 1;
      continue;
    }
    seenThreads.add(key);
    try {
      const thread = await deps.getThread(email.accountId, email.threadId);
      // A thread that is no longer listed still holds the email itself.
      const messages = thread.length > 0 ? thread : [email];
      for (const m of messages) {
        await deps.deleteEmail(m.accountId, m.id);
      }
      result.done += 1;
    } catch (err) {
      result.failed.push({ emailId: email.id, error: message(err) });
    }
  }
  return result;
}

/** Move every selected email to `targetMailbox`; already-there emails are skipped. */
export async function bulkMove(emails: Email[], targetMailbox: MailboxView, deps: BulkDeps): Promise<BulkResult> {
  const result: BulkResult = { done: 0, failed: [] };
  for (const email of emails) {
    if (email.mailbox === targetMailbox) {
      result.done += 1;
      continue;
    }
    try {
      await deps.moveEmail(email.accountId, email.id, targetMailbox);
      result.done += 1;
    } catch (err) {
      result.failed.push({ emailId: email.id, error: message(err) });
    }
  }
  return result;
}

/**
 * Whether a set of emails can be moved to a folder together: same IMAP
 * account (folders belong to one account) and every email in the inbox or a
 * custom folder — the same rule as the single-email "Move to folder".
 */
export function canBulkMove(emails: Email[], accountProvider: (accountId: string) => string | undefined): boolean {
  if (emails.length === 0) return false;
  const accountId = emails[0].accountId;
  if (accountProvider(accountId) !== 'imap') return false;
  return emails.every((e) => e.accountId === accountId && (e.mailbox === 'inbox' || e.mailbox.startsWith('folder:')));
}
