// "Delete forever" and "Empty Trash / Spam": which views and accounts offer
// it, the backend events, and the dialog's state machine.

import { describe, expect, it } from 'vitest';
import type { Account, Email } from '@/types';
import {
  bulkDeleteForever,
  type DeleteForeverFinished,
  deleteForeverFolderOf,
  deleteForeverReducer,
  deleteForeverSupport,
  failureReasons,
  isDeleteForeverFinished,
  isDeleteForeverProgress,
} from './deleteForever';

const row = (id: string, accountId = 'imap-1') => ({ id, accountId, mailbox: 'trash' }) as Email;
const account = (id: string, provider: string) => ({ id, provider }) as Account;

const finished = (extra: Partial<DeleteForeverFinished> = {}): DeleteForeverFinished => ({
  runId: 'run-1',
  accountId: 'imap-1',
  folder: 'trash',
  total: 3,
  removed: 3,
  failed: [],
  notAttempted: 0,
  cancelled: false,
  error: null,
  ...extra,
});

describe('deleteForeverFolderOf', () => {
  it('maps the Trash and Spam views to their folder and nothing else', () => {
    expect(deleteForeverFolderOf('deleted')).toBe('trash');
    expect(deleteForeverFolderOf('spam')).toBe('spam');
    expect(deleteForeverFolderOf('inbox')).toBeNull();
    expect(deleteForeverFolderOf('archive')).toBeNull();
    expect(deleteForeverFolderOf('folder:Projects')).toBeNull();
  });
});

describe('deleteForeverSupport', () => {
  it('is available on IMAP and Outlook', () => {
    expect(deleteForeverSupport('imap')).toBe('available');
    expect(deleteForeverSupport('outlook')).toBe('available');
  });

  it('explains Gmail apart: the app does not hold the scope', () => {
    expect(deleteForeverSupport('gmail')).toBe('gmail');
  });
});

describe('bulkDeleteForever', () => {
  const accounts = [account('imap-1', 'imap'), account('imap-2', 'imap'), account('g-1', 'gmail')];

  it('applies to rows of one supported account in the Trash or Spam view', () => {
    expect(bulkDeleteForever([row('a'), row('b')], accounts, 'deleted')).toEqual({
      accountId: 'imap-1',
      folder: 'trash',
    });
    expect(bulkDeleteForever([row('a')], accounts, 'spam')).toEqual({ accountId: 'imap-1', folder: 'spam' });
  });

  it('does not apply outside Trash and Spam', () => {
    expect(bulkDeleteForever([row('a')], accounts, 'inbox')).toBeNull();
  });

  it('does not apply to a selection spanning accounts', () => {
    expect(bulkDeleteForever([row('a'), row('b', 'imap-2')], accounts, 'deleted')).toBeNull();
  });

  it('does not apply to a Gmail account', () => {
    expect(bulkDeleteForever([row('a', 'g-1')], accounts, 'deleted')).toBeNull();
  });
});

describe('event payloads', () => {
  it('accepts a well-formed progress event and rejects a malformed one', () => {
    const progress = {
      runId: 'run-1',
      accountId: 'imap-1',
      folder: 'trash',
      processed: 1,
      total: 3,
      removed: 1,
      failed: 0,
    };
    expect(isDeleteForeverProgress(progress)).toBe(true);
    expect(isDeleteForeverProgress({ ...progress, processed: '1' })).toBe(false);
    expect(isDeleteForeverProgress(null)).toBe(false);
  });

  it('accepts a finished event with failures and an error, and rejects a malformed one', () => {
    const withFailures = finished({
      failed: [{ emailId: 'm-1', error: { code: 'sync', params: {}, message: 'kept' } }],
      error: { code: 'expunge_not_safe', params: {}, message: 'unsafe' },
    });
    expect(isDeleteForeverFinished(withFailures)).toBe(true);
    expect(isDeleteForeverFinished({ ...withFailures, failed: [{ emailId: 'm-1' }] })).toBe(false);
    expect(isDeleteForeverFinished({ ...finished(), folder: 'inbox' })).toBe(false);
  });
});

describe('deleteForeverReducer', () => {
  it('asks to confirm with the count once it is known', () => {
    expect(deleteForeverReducer({ phase: 'counting' }, { type: 'counted', count: 12 })).toEqual({
      phase: 'confirm',
      count: 12,
    });
  });

  it('shows progress of its own run only', () => {
    const running = deleteForeverReducer({ phase: 'confirm', count: 3 }, { type: 'started', runId: 'run-1' });
    expect(running).toEqual({ phase: 'running', runId: 'run-1', processed: 0, total: 3, cancelling: false });

    const moved = deleteForeverReducer(running, {
      type: 'progress',
      progress: { runId: 'run-1', accountId: 'imap-1', folder: 'trash', processed: 2, total: 3, removed: 2, failed: 0 },
    });
    expect(moved).toMatchObject({ processed: 2, total: 3 });

    const other = deleteForeverReducer(moved, {
      type: 'progress',
      progress: { runId: 'run-9', accountId: 'imap-1', folder: 'trash', processed: 9, total: 9, removed: 9, failed: 0 },
    });
    expect(other).toBe(moved);
  });

  it('marks a cancel request until the run reports how it ended', () => {
    const running = deleteForeverReducer({ phase: 'confirm', count: 3 }, { type: 'started', runId: 'run-1' });
    const cancelling = deleteForeverReducer(running, { type: 'cancelRequested' });
    expect(cancelling).toMatchObject({ phase: 'running', cancelling: true });

    const done = deleteForeverReducer(cancelling, {
      type: 'finished',
      report: finished({ removed: 2, notAttempted: 1, cancelled: true }),
    });
    expect(done).toEqual({ phase: 'done', report: finished({ removed: 2, notAttempted: 1, cancelled: true }) });
  });

  it('ignores the end of another run', () => {
    const running = deleteForeverReducer({ phase: 'confirm', count: 3 }, { type: 'started', runId: 'run-1' });
    expect(deleteForeverReducer(running, { type: 'finished', report: finished({ runId: 'run-2' }) })).toBe(running);
  });

  it('reports a run that could not start', () => {
    const running = deleteForeverReducer({ phase: 'confirm', count: 3 }, { type: 'started', runId: 'run-1' });
    expect(deleteForeverReducer(running, { type: 'failed', message: 'offline' })).toEqual({
      phase: 'failed',
      message: 'offline',
    });
  });
});

describe('failureReasons', () => {
  it('groups the failures by reason, most frequent first', () => {
    const fail = (emailId: string, code: string) => ({ emailId, error: { code, params: {}, message: code } });
    const reasons = failureReasons([fail('a', 'sync'), fail('b', 'not_found'), fail('c', 'sync')], (e) => e.message);
    expect(reasons).toEqual([
      { text: 'sync', count: 2 },
      { text: 'not_found', count: 1 },
    ]);
  });
});
