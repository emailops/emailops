// Undo send and scheduled send: the store's send entry point, the Undo toast
// race and the Scheduled view's actions, around a mocked backend.

import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  queueOutgoingEmail: vi.fn(),
  cancelOutboxMessage: vi.fn(),
  sendOutboxMessageNow: vi.fn(async () => {}),
  listOutbox: vi.fn(async () => []),
}));

import { initI18n } from '@/i18n';
import type { OutboxEntry, OutgoingMessage } from '@/lib/api';
import * as api from '@/lib/api';
import { useOutboxStore, withoutEntry } from './outboxStore';
import { useToastStore } from './toastStore';

const message: OutgoingMessage = {
  accountId: 'acc',
  to: ['ana@example.com'],
  cc: [],
  subject: 'Plans',
  body: 'See you',
  bodyHtml: '<p>See you</p>',
  inlineImages: [],
  attachments: [],
};

const entry = (id: string, extra: Partial<OutboxEntry> = {}): OutboxEntry => ({
  id,
  accountId: 'acc',
  kind: 'new',
  replyToEmailId: null,
  origin: 'undo',
  toAddresses: ['ana@example.com'],
  ccAddresses: [],
  subject: 'Plans',
  attachmentCount: 0,
  sendAt: 2_000,
  status: 'scheduled',
  attempts: 0,
  lastError: null,
  failureKind: null,
  createdAt: 1_000,
  ...extra,
});

const lastToast = () => {
  const toasts = useToastStore.getState().toasts;
  return toasts[toasts.length - 1];
};

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState({ toasts: [], nextId: 1 });
  useOutboxStore.setState({ entries: [], undoDelaySecs: null, scope: null });
  vi.mocked(api.getPref).mockResolvedValue(null);
  vi.mocked(api.queueOutgoingEmail).mockResolvedValue(entry('o1'));
  vi.mocked(api.cancelOutboxMessage).mockResolvedValue(message);
});

describe('send', () => {
  it('sends directly when undo send is off', async () => {
    vi.mocked(api.getPref).mockResolvedValue('0');
    const sendDirect = vi.fn(async () => {});
    const outcome = await useOutboxStore.getState().send(message, { sendDirect });
    expect(outcome).toBe('sent');
    expect(sendDirect).toHaveBeenCalledOnce();
    expect(api.queueOutgoingEmail).not.toHaveBeenCalled();
  });

  it('queues for the undo window (10 s by default) and offers Undo for that long', async () => {
    const sendDirect = vi.fn(async () => {});
    const outcome = await useOutboxStore.getState().send(message, { sendDirect, draftId: 'd1' });
    expect(outcome).toBe('queued');
    expect(sendDirect).not.toHaveBeenCalled();
    expect(api.queueOutgoingEmail).toHaveBeenCalledWith(message, { type: 'undo', delaySecs: 10 }, 'd1');
    const toast = lastToast();
    expect(toast.durationMs).toBe(10_000);
    expect(toast.actionLabel).toBe('Undo');
  });

  it('Undo takes the message back and reopens the composer with it', async () => {
    const restore = vi.fn();
    useOutboxStore.getState().setRestoreHandler(restore);
    await useOutboxStore.getState().send(message, { sendDirect: vi.fn() });
    lastToast().onAction?.();
    await vi.waitFor(() => expect(restore).toHaveBeenCalledWith(message));
    expect(api.cancelOutboxMessage).toHaveBeenCalledWith('o1');
  });

  it('a late Undo (already sending) says so and reopens nothing', async () => {
    const restore = vi.fn();
    useOutboxStore.getState().setRestoreHandler(restore);
    vi.mocked(api.cancelOutboxMessage).mockRejectedValue({
      code: 'outbox_not_pending',
      params: {},
      message: 'already being sent',
    });
    await useOutboxStore.getState().send(message, { sendDirect: vi.fn() });
    lastToast().onAction?.();
    await vi.waitFor(() => expect(lastToast().message).toMatch(/already/i));
    expect(restore).not.toHaveBeenCalled();
  });
});

describe('schedule', () => {
  it('queues at the chosen time and confirms with an Undo', async () => {
    vi.mocked(api.queueOutgoingEmail).mockResolvedValue(entry('s1', { origin: 'scheduled', sendAt: 1_900_000_000 }));
    const at = new Date(1_900_000_000 * 1000);
    await useOutboxStore.getState().schedule(message, at, 'd1');
    expect(api.queueOutgoingEmail).toHaveBeenCalledWith(message, { type: 'at', sendAt: 1_900_000_000 }, 'd1');
    expect(lastToast().actionLabel).toBe('Undo');
    expect(api.listOutbox).toHaveBeenCalled();
  });
});

describe('Scheduled view actions', () => {
  it('delete takes the row out and its Undo reopens the message', async () => {
    const restore = vi.fn();
    useOutboxStore.getState().setRestoreHandler(restore);
    useOutboxStore.setState({ entries: [entry('a'), entry('b')] });
    await useOutboxStore.getState().remove('a');
    expect(useOutboxStore.getState().entries.map((e) => e.id)).toEqual(['b']);
    lastToast().onAction?.();
    expect(restore).toHaveBeenCalledWith(message);
  });

  it('edit cancels the row and reopens the composer', async () => {
    const restore = vi.fn();
    useOutboxStore.getState().setRestoreHandler(restore);
    useOutboxStore.setState({ entries: [entry('a')] });
    await useOutboxStore.getState().edit('a');
    expect(restore).toHaveBeenCalledWith(message);
    expect(useOutboxStore.getState().entries).toEqual([]);
  });

  it('send now asks the backend and refreshes', async () => {
    await useOutboxStore.getState().sendNow('a');
    expect(api.sendOutboxMessageNow).toHaveBeenCalledWith('a');
    expect(api.listOutbox).toHaveBeenCalled();
  });
});

describe('outbox-updated', () => {
  it('a failure is announced with a way to the Scheduled view', () => {
    useOutboxStore
      .getState()
      .applyUpdate({ sent: [], failed: [{ id: 'f', accountId: 'acc', interrupted: false, message: '503' }] });
    expect(lastToast().message).toMatch(/could not be sent/i);
    expect(lastToast().sticky).toBe(true);
  });

  it('an interrupted send tells the user to check Sent', () => {
    useOutboxStore
      .getState()
      .applyUpdate({ sent: [], failed: [{ id: 'f', accountId: 'acc', interrupted: true, message: 'stopped' }] });
    expect(lastToast().message).toMatch(/Sent/);
  });

  it('a sent row leaves the list', () => {
    useOutboxStore.setState({ entries: [entry('a'), entry('b')] });
    useOutboxStore.getState().applyUpdate({ sent: [{ id: 'a', accountId: 'acc', threadId: null }], failed: [] });
    expect(useOutboxStore.getState().entries.map((e) => e.id)).toEqual(['b']);
  });
});

describe('withoutEntry', () => {
  it('drops one row and keeps the others in order', () => {
    expect(withoutEntry([entry('a'), entry('b'), entry('c')], 'b').map((e) => e.id)).toEqual(['a', 'c']);
    expect(withoutEntry([entry('a')], 'zz').map((e) => e.id)).toEqual(['a']);
  });
});
