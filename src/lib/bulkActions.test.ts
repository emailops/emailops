import { describe, expect, it, vi } from 'vitest';
import type { Email } from '@/types';
import { type BulkDeps, bulkDelete, bulkMove, canBulkMove } from './bulkActions';

function mail(id: string, threadId = id, over: Partial<Email> = {}): Email {
  return { id, threadId, accountId: 'a1', mailbox: 'inbox', ...over } as Email;
}

function deps(over: Partial<BulkDeps> = {}): BulkDeps & {
  getThread: ReturnType<typeof vi.fn>;
  deleteEmail: ReturnType<typeof vi.fn>;
  moveEmail: ReturnType<typeof vi.fn>;
} {
  return {
    getThread: vi.fn(async (_acc: string, threadId: string) => [
      mail(`${threadId}-1`, threadId),
      mail(`${threadId}-2`, threadId),
    ]),
    deleteEmail: vi.fn(async () => {}),
    moveEmail: vi.fn(async () => {}),
    ...over,
  } as never;
}

describe('bulkDelete', () => {
  it('moves every message of each selected thread to the Trash, like the row menu', async () => {
    const d = deps();
    const r = await bulkDelete([mail('x', 't1'), mail('y', 't2')], d);
    expect(r).toEqual({ done: 2, failed: [] });
    expect(d.deleteEmail.mock.calls.map((c) => c[1])).toEqual(['t1-1', 't1-2', 't2-1', 't2-2']);
  });

  it('deletes a thread only once when several of its messages are selected', async () => {
    const d = deps();
    const r = await bulkDelete([mail('x', 't1'), mail('y', 't1')], d);
    expect(r.done).toBe(2);
    expect(d.getThread).toHaveBeenCalledTimes(1);
    expect(d.deleteEmail).toHaveBeenCalledTimes(2);
  });

  it('falls back to the email itself when its thread comes back empty', async () => {
    const d = deps({ getThread: vi.fn(async () => []) });
    await bulkDelete([mail('x', 't1')], d);
    expect(d.deleteEmail).toHaveBeenCalledWith('a1', 'x');
  });

  it('keeps going after a failure and reports it', async () => {
    const d = deps({
      deleteEmail: vi.fn(async (_a: string, id: string) => {
        if (id.startsWith('t1')) throw new Error('IMAP offline');
      }),
    });
    const r = await bulkDelete([mail('x', 't1'), mail('y', 't2')], d);
    expect(r.done).toBe(1);
    expect(r.failed).toEqual([{ emailId: 'x', error: 'IMAP offline' }]);
    expect(d.deleteEmail.mock.calls.some((c) => c[1] === 't2-1')).toBe(true);
  });
});

describe('bulkMove', () => {
  it('moves each selected email and skips those already in the target', async () => {
    const d = deps();
    const r = await bulkMove(
      [mail('x'), mail('y', 'y', { mailbox: 'folder:INBOX.Archive' }), mail('z')],
      'folder:INBOX.Archive',
      d,
    );
    expect(r).toEqual({ done: 3, failed: [] });
    expect(d.moveEmail.mock.calls).toEqual([
      ['a1', 'x', 'folder:INBOX.Archive'],
      ['a1', 'z', 'folder:INBOX.Archive'],
    ]);
  });

  it('reports failures without stopping', async () => {
    const d = deps({
      moveEmail: vi.fn(async (_a: string, id: string) => {
        if (id === 'x') throw 'busy';
      }),
    });
    const r = await bulkMove([mail('x'), mail('y')], 'folder:INBOX.A', d);
    expect(r).toEqual({ done: 1, failed: [{ emailId: 'x', error: 'busy' }] });
  });
});

describe('canBulkMove', () => {
  const imap = () => 'imap';
  it('allows inbox / custom-folder emails of one IMAP account', () => {
    expect(canBulkMove([mail('x'), mail('y', 'y', { mailbox: 'folder:INBOX.A' })], imap)).toBe(true);
  });
  it('refuses mixed accounts, other providers, other mailboxes, or nothing', () => {
    expect(canBulkMove([mail('x'), mail('y', 'y', { accountId: 'a2' })], imap)).toBe(false);
    expect(canBulkMove([mail('x')], () => 'gmail')).toBe(false);
    expect(canBulkMove([mail('x', 'x', { mailbox: 'sent' })], imap)).toBe(false);
    expect(canBulkMove([], imap)).toBe(false);
  });
});
