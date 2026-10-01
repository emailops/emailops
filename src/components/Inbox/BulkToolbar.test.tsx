// The bulk toolbar shown over the list while rows are selected: which actions
// it offers, and that they act on the selected conversations.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  applyThreadAction: vi.fn(async () => ({ failed: [] })),
  moveEmails: vi.fn(async () => ({ failed: [] })),
}));

import { useAccountStore } from '@/stores/accountStore';
import { snoozeMap, useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { useSelectionStore } from '@/stores/selectionStore';
import { BulkToolbar } from './BulkToolbar';

const row = (id: string, extra: Partial<Email> = {}) =>
  ({
    id,
    accountId: 'acc',
    threadId: `t-${id}`,
    mailbox: 'inbox',
    isRead: true,
    isStarred: false,
    subject: id,
    timestamp: 1,
    ...extra,
  }) as Email;

let container: HTMLDivElement;
let root: Root;
const deleteThreads = vi.fn(async () => undefined);
const archiveThreads = vi.fn(async () => undefined);
const setThreadsRead = vi.fn(async () => undefined);
const moveEmailsToMailbox = vi.fn(async () => undefined);
const snoozeThreads = vi.fn(async () => undefined);
const unsnoozeThreads = vi.fn(async () => undefined);

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
  useSelectionStore.getState().clear();
  useEmailStore.setState({
    deleteThreads,
    archiveThreads,
    setThreadsRead,
    moveEmailsToMailbox,
    snoozeThreads,
    unsnoozeThreads,
    snoozes: new Map(),
    listScope: 'inbox',
  });
  useAccountStore.setState({ accounts: [] });
  useFolderStore.setState({ folders: [], accountId: null });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(emails: Email[]) {
  act(() => root.render(<BulkToolbar emails={emails} />));
}

const q = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);

async function click(id: string) {
  const el = q(id);
  if (!el) throw new Error(`${id} not rendered`);
  await act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

describe('BulkToolbar', () => {
  it('deletes the selected conversations in one call and clears the selection', async () => {
    const emails = [row('a'), row('b'), row('c')];
    useSelectionStore.getState().selectAll(['a', 'c']);
    render(emails);

    await click('bulk-delete');

    expect(deleteThreads).toHaveBeenCalledTimes(1);
    expect(deleteThreads).toHaveBeenCalledWith([
      { accountId: 'acc', threadId: 't-a' },
      { accountId: 'acc', threadId: 't-c' },
    ]);
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('offers archive only for inbox mail', () => {
    useSelectionStore.getState().selectAll(['a']);
    render([row('a', { mailbox: 'archive' })]);
    expect(q('bulk-archive')).toBeNull();
    expect(q('bulk-delete')).not.toBeNull();

    render([row('a')]);
    expect(q('bulk-archive')).not.toBeNull();
  });

  it('marks read and keeps the selection', async () => {
    useSelectionStore.getState().selectAll(['a']);
    render([row('a', { isRead: false })]);

    await click('bulk-mark-read');

    expect(setThreadsRead).toHaveBeenCalledWith([{ accountId: 'acc', threadId: 't-a' }], true);
    expect(useSelectionStore.getState().ids.size).toBe(1);
  });

  it('the header checkbox selects every loaded row, then clears', async () => {
    useSelectionStore.getState().selectAll(['a']);
    render([row('a'), row('b')]);

    await click('bulk-select-all');
    expect([...useSelectionStore.getState().ids].sort()).toEqual(['a', 'b']);

    await click('bulk-select-all');
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('moves IMAP messages to a folder in one call', async () => {
    useAccountStore.setState({ accounts: [{ id: 'acc', provider: 'imap' }] as never });
    useFolderStore.setState({
      accountId: 'acc',
      folders: [
        { id: 'f', accountId: 'acc', serverPath: 'Projects', displayName: 'Projects', role: 'custom', delimiter: '/' },
      ],
    });
    useSelectionStore.getState().selectAll(['a', 'b']);
    render([row('a'), row('b')]);

    await click('bulk-move');
    await click('bulk-move-folder:Projects');

    expect(moveEmailsToMailbox).toHaveBeenCalledWith('acc', ['a', 'b'], 'folder:Projects');
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('snoozes the selected inbox conversations and clears the selection', async () => {
    useSelectionStore.getState().selectAll(['a', 'b']);
    render([row('a'), row('b')]);

    await click('bulk-snooze');
    await click('snooze-preset-tomorrow');

    expect(snoozeThreads).toHaveBeenCalledTimes(1);
    const [threads, until] = snoozeThreads.mock.calls[0] as unknown as [unknown, number];
    expect(threads).toEqual([
      { accountId: 'acc', threadId: 't-a' },
      { accountId: 'acc', threadId: 't-b' },
    ]);
    expect(until).toBeGreaterThan(Date.now() / 1000);
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('unsnoozes the snoozed conversations of the selection', async () => {
    useEmailStore.setState({
      listScope: 'snoozed',
      snoozes: snoozeMap([{ accountId: 'acc', threadId: 't-a', snoozedUntil: 9e9, createdAt: 1, wokeAt: null }]),
    });
    useSelectionStore.getState().selectAll(['a', 'b']);
    render([row('a'), row('b')]);

    await click('bulk-unsnooze');

    expect(unsnoozeThreads).toHaveBeenCalledWith([{ accountId: 'acc', threadId: 't-a' }]);
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('renders nothing without a selection', () => {
    render([row('a')]);
    expect(q('bulk-toolbar')).toBeNull();
  });
});
