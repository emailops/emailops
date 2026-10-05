import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { BlockedSender, Email, SenderMoveReport, SenderStatus, UnsubscribeKind } from '@/types';
import { removeThreads, threadKey, useEmailStore } from './emailStore';
import { useLogStore } from './logStore';

/**
 * Block sender and one-click unsubscribe: the sender facts of opened messages
 * (cached per message), the blocked-senders list for Settings, and the one
 * pending confirmation dialog (`SenderDialogs` renders it).
 */

/** The confirmation the user is being asked for. */
export type SenderDialogRequest =
  | { type: 'unsubscribe'; accountId: string; emailId: string; senderName: string }
  | { type: 'block'; accountId: string; address: string }
  | { type: 'unblock'; accountId: string; address: string };

interface SenderStore {
  statusByEmail: Record<string, SenderStatus>;
  blocked: BlockedSender[];
  dialog: SenderDialogRequest | null;

  /** Fetch one message's sender facts (logged, never thrown, on failure). */
  loadStatus: (accountId: string, emailId: string) => Promise<void>;
  loadBlocked: () => Promise<void>;
  openDialog: (request: SenderDialogRequest) => void;
  closeDialog: () => void;
  /** Rejects on failure; on success every cached message of that sender shows "unsubscribed". */
  unsubscribe: (accountId: string, emailId: string) => Promise<UnsubscribeKind>;
  /** Rejects on failure. Moved conversations leave the visible list. */
  block: (accountId: string, address: string, moveExisting: boolean) => Promise<SenderMoveReport>;
  unblock: (accountId: string, address: string, restore: boolean) => Promise<SenderMoveReport>;
}

/** Cache key of one message's sender facts. */
export function statusKey(accountId: string, emailId: string): string {
  return `${accountId}\u0000${emailId}`;
}

function sameSender(status: SenderStatus, key: string, accountId: string, address: string): boolean {
  return key.startsWith(`${accountId}\u0000`) && status.address === address.toLowerCase();
}

/** Pure: every cached message of `address` in `accountId` patched with `patch`. */
export function patchSender(
  byEmail: Record<string, SenderStatus>,
  accountId: string,
  address: string,
  patch: Partial<SenderStatus>,
): Record<string, SenderStatus> {
  const out: Record<string, SenderStatus> = {};
  for (const [key, status] of Object.entries(byEmail)) {
    out[key] = sameSender(status, key, accountId, address) ? { ...status, ...patch } : status;
  }
  return out;
}

/** Pure: the conversations of the list whose row comes from `address` and that
 *  a block filed in Spam (inbox and archive rows). */
export function blockedThreadKeys(rows: readonly Email[], accountId: string, address: string): Set<string> {
  const target = address.toLowerCase();
  return new Set(
    rows
      .filter(
        (e) =>
          e.accountId === accountId &&
          e.senderEmail.trim().toLowerCase() === target &&
          (e.mailbox === 'inbox' || e.mailbox === 'archive'),
      )
      .map((e) => threadKey(e.accountId, e.threadId)),
  );
}

export const useSenderStore = create<SenderStore>((set, get) => ({
  statusByEmail: {},
  blocked: [],
  dialog: null,

  loadStatus: async (accountId, emailId) => {
    try {
      const status = await api.getSenderStatus(accountId, emailId);
      set((state) => ({ statusByEmail: { ...state.statusByEmail, [statusKey(accountId, emailId)]: status } }));
    } catch (err) {
      useLogStore.getState().addLog('error', 'account', `Could not load the sender details: ${errorText(err)}`);
    }
  },

  loadBlocked: async () => {
    try {
      set({ blocked: await api.listBlockedSenders(null) });
    } catch (err) {
      useLogStore.getState().addLog('error', 'account', `Could not load the blocked senders: ${errorText(err)}`);
    }
  },

  openDialog: (request) => set({ dialog: request }),
  closeDialog: () => set({ dialog: null }),

  unsubscribe: async (accountId, emailId) => {
    const kind = await api.unsubscribeFromSender(accountId, emailId);
    const status = get().statusByEmail[statusKey(accountId, emailId)];
    if (status) {
      const now = Math.floor(Date.now() / 1000);
      set((state) => ({
        statusByEmail: patchSender(state.statusByEmail, accountId, status.address, { unsubscribedAt: now }),
      }));
    }
    return kind;
  },

  block: async (accountId, address, moveExisting) => {
    const report = await api.blockSender(accountId, address, moveExisting);
    set((state) => ({ statusByEmail: patchSender(state.statusByEmail, accountId, address, { blocked: true }) }));
    if (report.moved > 0) {
      const keys = blockedThreadKeys(useEmailStore.getState().emails, accountId, address);
      if (keys.size > 0) useEmailStore.setState((state) => removeThreads(state, keys));
    }
    await get().loadBlocked();
    return report;
  },

  unblock: async (accountId, address, restore) => {
    const report = await api.unblockSender(accountId, address, restore);
    set((state) => ({ statusByEmail: patchSender(state.statusByEmail, accountId, address, { blocked: false }) }));
    await get().loadBlocked();
    return report;
  },
}));
