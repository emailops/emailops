// "Delete forever" on messages in Trash or Spam, and "Empty Trash / Spam"
// (issue #304). The backend run is `delete_forever`; it reports
// `delete-forever-progress` and one `delete-forever-finished`, tagged with the
// run id the dialog chose.

import type { MailboxView } from '@/lib/api';
import { type AppErrorPayload, isAppErrorPayload } from '@/lib/errors';
import type { Account, Email } from '@/types';

export type DeleteForeverFolder = 'trash' | 'spam';

/** The folder a list view shows, when it is one that can be emptied. */
export function deleteForeverFolderOf(view: MailboxView): DeleteForeverFolder | null {
  if (view === 'deleted') return 'trash';
  if (view === 'spam') return 'spam';
  return null;
}

/**
 * Whether an account's provider can delete forever. Gmail cannot: its
 * `messages.delete` needs full Gmail access (`https://mail.google.com/`),
 * which EmailOps does not request; Gmail empties Trash and Spam itself after
 * 30 days. Mirrors `provider_supports_delete_forever` in the backend.
 */
export function deleteForeverSupport(provider: string): 'available' | 'gmail' | 'unavailable' {
  if (provider === 'imap' || provider === 'outlook') return 'available';
  if (provider === 'gmail') return 'gmail';
  return 'unavailable';
}

/** The account and folder a bulk "Delete forever" applies to: the Trash or
 *  Spam view, every row of one account, and an account that supports it. */
export function bulkDeleteForever(
  rows: readonly Email[],
  accounts: readonly Pick<Account, 'id' | 'provider'>[],
  view: MailboxView,
): { accountId: string; folder: DeleteForeverFolder } | null {
  const folder = deleteForeverFolderOf(view);
  if (!folder || rows.length === 0) return null;
  const accountId = rows[0].accountId;
  if (rows.some((e) => e.accountId !== accountId)) return null;
  const provider = accounts.find((a) => a.id === accountId)?.provider;
  if (!provider || deleteForeverSupport(provider) !== 'available') return null;
  return { accountId, folder };
}

export interface DeleteForeverProgress {
  runId: string;
  accountId: string;
  folder: DeleteForeverFolder;
  processed: number;
  total: number;
  removed: number;
  failed: number;
}

export interface DeleteForeverFailure {
  emailId: string;
  error: AppErrorPayload;
}

export interface DeleteForeverFinished {
  runId: string;
  accountId: string;
  folder: DeleteForeverFolder;
  total: number;
  removed: number;
  failed: DeleteForeverFailure[];
  notAttempted: number;
  cancelled: boolean;
  error: AppErrorPayload | null;
}

function isFolder(v: unknown): v is DeleteForeverFolder {
  return v === 'trash' || v === 'spam';
}

function isRunTag(o: Record<string, unknown>): boolean {
  return typeof o.runId === 'string' && typeof o.accountId === 'string' && isFolder(o.folder);
}

export function isDeleteForeverProgress(p: unknown): p is DeleteForeverProgress {
  if (typeof p !== 'object' || p === null) return false;
  const o = p as Record<string, unknown>;
  return (
    isRunTag(o) &&
    typeof o.processed === 'number' &&
    typeof o.total === 'number' &&
    typeof o.removed === 'number' &&
    typeof o.failed === 'number'
  );
}

function isFailure(f: unknown): f is DeleteForeverFailure {
  if (typeof f !== 'object' || f === null) return false;
  const o = f as Record<string, unknown>;
  return typeof o.emailId === 'string' && isAppErrorPayload(o.error);
}

export function isDeleteForeverFinished(p: unknown): p is DeleteForeverFinished {
  if (typeof p !== 'object' || p === null) return false;
  const o = p as Record<string, unknown>;
  return (
    isRunTag(o) &&
    typeof o.total === 'number' &&
    typeof o.removed === 'number' &&
    Array.isArray(o.failed) &&
    o.failed.every(isFailure) &&
    typeof o.notAttempted === 'number' &&
    typeof o.cancelled === 'boolean' &&
    (o.error === null || o.error === undefined || isAppErrorPayload(o.error))
  );
}

/** Why messages are still there, once per reason, most frequent first. */
export function failureReasons(
  failed: readonly DeleteForeverFailure[],
  text: (error: AppErrorPayload) => string,
): { text: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const failure of failed) {
    const reason = text(failure.error);
    counts.set(reason, (counts.get(reason) ?? 0) + 1);
  }
  return [...counts].map(([reason, count]) => ({ text: reason, count })).sort((a, b) => b.count - a.count);
}

export type DeleteForeverState =
  | { phase: 'counting' }
  | { phase: 'confirm'; count: number }
  | { phase: 'running'; runId: string; processed: number; total: number; cancelling: boolean }
  | { phase: 'done'; report: DeleteForeverFinished }
  | { phase: 'failed'; message: string };

export type DeleteForeverAction =
  | { type: 'counted'; count: number }
  | { type: 'started'; runId: string }
  | { type: 'progress'; progress: DeleteForeverProgress }
  | { type: 'cancelRequested' }
  | { type: 'finished'; report: DeleteForeverFinished }
  | { type: 'failed'; message: string };

/** The dialog's state machine: count → confirm → running → done, or failed
 *  when the count or the start was refused. Events of another run are
 *  ignored. */
export function deleteForeverReducer(state: DeleteForeverState, action: DeleteForeverAction): DeleteForeverState {
  switch (action.type) {
    case 'counted':
      return state.phase === 'counting' ? { phase: 'confirm', count: action.count } : state;
    case 'started':
      return state.phase === 'confirm'
        ? { phase: 'running', runId: action.runId, processed: 0, total: state.count, cancelling: false }
        : state;
    case 'progress':
      return state.phase === 'running' && action.progress.runId === state.runId
        ? { ...state, processed: action.progress.processed, total: action.progress.total }
        : state;
    case 'cancelRequested':
      return state.phase === 'running' ? { ...state, cancelling: true } : state;
    case 'finished':
      return state.phase === 'running' && action.report.runId === state.runId
        ? { phase: 'done', report: action.report }
        : state;
    case 'failed':
      return { phase: 'failed', message: action.message };
  }
}
