// The auto-advance executor: after the open conversation leaves the list, open
// its neighbour — but never touch a conversation the user opened meanwhile.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  getThread: vi.fn(async () => []),
  getEmailBody: vi.fn(async () => ''),
  markAsRead: vi.fn(async () => {}),
  applyThreadAction: vi.fn(async () => ({ failed: [] })),
}));

import * as api from '@/lib/api';
import { AFTER_LEAVE_PREF } from '@/lib/autoAdvance';
import type { Email } from '@/types';
import { beginLeave, finishLeave, useAutoAdvanceStore } from './autoAdvanceStore';
import { removeThreads, threadKey, useEmailStore } from './emailStore';
import { useShortcutStore } from './shortcutStore';

const row = (id: string, threadId = `t-${id}`): Email =>
  ({ id, accountId: 'a1', threadId, mailbox: 'inbox', isRead: true, subject: id, timestamp: 1 }) as Email;

const LIST = [row('e1'), row('e2'), row('e3')];
const close = vi.fn();

async function open(email: Email | null) {
  await useEmailStore.getState().selectEmail(email);
}

/** What an archive does to the store: the thread leaves the list. */
function removeThread(email: Email) {
  useEmailStore.setState((s) => removeThreads(s, new Set([threadKey(email.accountId, email.threadId)])));
}

beforeEach(async () => {
  close.mockClear();
  useAutoAdvanceStore.setState({ mode: 'next' });
  useEmailStore.setState({ emails: LIST, totalCount: 3, tabs: [], activeTabId: null });
  useShortcutStore.setState({ listEmails: LIST, cursorId: 'e2' });
  await open(LIST[1]);
});

afterEach(() => {
  vi.mocked(api.setPref).mockReset();
});

describe('finishLeave', () => {
  it('opens the next conversation after the open one leaves', () => {
    const ticket = beginLeave();
    removeThread(LIST[1]);
    expect(finishLeave(ticket, { close })).toBe('opened');
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e3');
    expect(useShortcutStore.getState().cursorId).toBe('e3');
    expect(close).not.toHaveBeenCalled();
  });

  it('opens the previous one when set to previous', () => {
    useAutoAdvanceStore.setState({ mode: 'previous' });
    const ticket = beginLeave();
    removeThread(LIST[1]);
    finishLeave(ticket, { close });
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e1');
  });

  it('goes back to the list when set to list', () => {
    useAutoAdvanceStore.setState({ mode: 'list' });
    const ticket = beginLeave();
    removeThread(LIST[1]);
    expect(finishLeave(ticket, { close })).toBe('closed');
    expect(close).toHaveBeenCalledTimes(1);
  });

  it('goes back to the list when nothing is left', async () => {
    useEmailStore.setState({ emails: [LIST[0]] });
    useShortcutStore.setState({ listEmails: [LIST[0]] });
    await open(LIST[0]);
    const ticket = beginLeave();
    removeThread(LIST[0]);
    expect(finishLeave(ticket, { close })).toBe('closed');
  });

  it('skips rows that left with it (a block moves all of a sender’s mail)', () => {
    const ticket = beginLeave();
    removeThread(LIST[1]);
    removeThread(LIST[2]);
    finishLeave(ticket, { close });
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e1');
  });

  it('does not close or replace a conversation the user opened while the action ran', async () => {
    const ticket = beginLeave();
    await open(LIST[0]);
    removeThread(LIST[1]);
    expect(finishLeave(ticket, { close })).toBe('stale');
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e1');
    expect(close).not.toHaveBeenCalled();
  });

  it('does not reopen anything after the user went back to the list meanwhile', async () => {
    const ticket = beginLeave();
    await open(null);
    expect(finishLeave(ticket, { close })).toBe('stale');
    expect(useEmailStore.getState().selectedEmail).toBeNull();
  });

  it('onlyIfLeft: stays on the conversation when it did not leave', () => {
    const ticket = beginLeave();
    expect(finishLeave(ticket, { close, onlyIfLeft: true })).toBe('stayed');
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e2');
    expect(close).not.toHaveBeenCalled();
  });

  it('onlyIfLeft: with nothing open there is nothing to leave', async () => {
    await open(null);
    const ticket = beginLeave();
    expect(finishLeave(ticket, { close, onlyIfLeft: true })).toBe('stayed');
    expect(close).not.toHaveBeenCalled();
  });

  it('a conversation shown in a tab just closes', () => {
    useEmailStore.setState({ activeTabId: 't-e2' });
    const ticket = beginLeave();
    removeThread(LIST[1]);
    expect(finishLeave(ticket, { close })).toBe('closed');
  });
});

describe('the preference', () => {
  it('loads from SQLite, defaulting to next', async () => {
    vi.mocked(api.getPref).mockResolvedValueOnce('previous');
    await useAutoAdvanceStore.getState().loadMode();
    expect(useAutoAdvanceStore.getState().mode).toBe('previous');
    vi.mocked(api.getPref).mockResolvedValueOnce(null);
    await useAutoAdvanceStore.getState().loadMode();
    expect(useAutoAdvanceStore.getState().mode).toBe('next');
  });

  it('saves, and keeps the old value when saving fails', async () => {
    vi.mocked(api.setPref).mockResolvedValueOnce(undefined);
    await useAutoAdvanceStore.getState().setMode('list');
    expect(api.setPref).toHaveBeenCalledWith(AFTER_LEAVE_PREF, 'list');
    expect(useAutoAdvanceStore.getState().mode).toBe('list');
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('disk full'));
    await expect(useAutoAdvanceStore.getState().setMode('next')).rejects.toThrow('disk full');
    expect(useAutoAdvanceStore.getState().mode).toBe('list');
  });
});
