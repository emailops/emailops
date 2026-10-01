import type { Folder, MailboxView } from '@/lib/api';
import type { Account, Email } from '@/types';

/** Which bulk actions apply to the selected rows. Archive follows the row
 *  menu's rule (inbox mail); the flag actions show only when they would
 *  change something. Delete always applies. */
export interface BulkAvailability {
  canArchive: boolean;
  canMarkRead: boolean;
  canMarkUnread: boolean;
  canStar: boolean;
  canUnstar: boolean;
}

export function bulkAvailability(rows: readonly Email[]): BulkAvailability {
  return {
    canArchive: rows.some((e) => e.mailbox === 'inbox'),
    canMarkRead: rows.some((e) => !e.isRead),
    canMarkUnread: rows.some((e) => e.isRead),
    canStar: rows.some((e) => !e.isStarred),
    canUnstar: rows.some((e) => e.isStarred),
  };
}

/**
 * Where the selected rows can be moved together, or null when a bulk move
 * does not apply. Same rules as the row menu (`useMoveTargets`): IMAP only,
 * inbox and custom-folder mail only, and the folder list loaded for that
 * account — so every row must belong to one account.
 */
export function bulkMoveTargets(
  rows: readonly Email[],
  accounts: readonly Pick<Account, 'id' | 'provider'>[],
  folders: readonly Folder[],
  foldersAccountId: string | null,
): { accountId: string; mailboxes: MailboxView[] } | null {
  if (rows.length === 0) return null;
  const accountId = rows[0].accountId;
  if (rows.some((e) => e.accountId !== accountId)) return null;
  if (accounts.find((a) => a.id === accountId)?.provider !== 'imap') return null;
  if (foldersAccountId !== accountId) return null;
  if (rows.some((e) => e.mailbox !== 'inbox' && !e.mailbox.startsWith('folder:'))) return null;
  const all = (mailbox: string) => rows.every((e) => e.mailbox === mailbox);
  const candidates: MailboxView[] = ['inbox', ...folders.map((f) => `folder:${f.serverPath}` as MailboxView)];
  return { accountId, mailboxes: candidates.filter((m) => !all(m)) };
}
