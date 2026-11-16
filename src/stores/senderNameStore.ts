import { create } from 'zustand';
import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';

/**
 * The name each address uses when it writes to an account, keyed by account
 * and lowercased address. Inbox rows use it to name the recipients of the
 * user's own messages, whose stored headers often hold only the address.
 *
 * One map per account, loaded once per session and again when the account
 * finishes a sync — never per row or per page.
 */
export interface SenderNameState {
  namesByAccount: Record<string, Record<string, string>>;
  /** The request in flight per account: an older one's answer is dropped. */
  requests: Record<string, number>;
}

export const INITIAL_SENDER_NAMES: SenderNameState = { namesByAccount: {}, requests: {} };

export function reduceRequested(state: SenderNameState, accountId: string, requestId: number): SenderNameState {
  return { ...state, requests: { ...state.requests, [accountId]: requestId } };
}

function withoutRequest(requests: Record<string, number>, accountId: string): Record<string, number> {
  const { [accountId]: _done, ...rest } = requests;
  return rest;
}

export function reduceLoaded(
  state: SenderNameState,
  accountId: string,
  requestId: number,
  names: Record<string, string>,
): SenderNameState {
  if (state.requests[accountId] !== requestId) return state;
  return {
    namesByAccount: { ...state.namesByAccount, [accountId]: names },
    requests: withoutRequest(state.requests, accountId),
  };
}

/** A failed load leaves the names it had; an account that never loaded gets
 *  an empty map, so rows show addresses and it is not asked again until its
 *  next sync. */
export function reduceFailed(state: SenderNameState, accountId: string, requestId: number): SenderNameState {
  if (state.requests[accountId] !== requestId) return state;
  return {
    namesByAccount: { ...state.namesByAccount, [accountId]: state.namesByAccount[accountId] ?? {} },
    requests: withoutRequest(state.requests, accountId),
  };
}

/** Accounts in `before` but not in `after`: those whose sync just ended. */
export function finishedSyncing(before: ReadonlySet<string>, after: ReadonlySet<string>): string[] {
  return [...before].filter((id) => !after.has(id));
}

interface SenderNameStore extends SenderNameState {
  /** Load an account's names unless they are loaded or loading. */
  ensureLoaded: (accountId: string) => Promise<void>;
  /** Fetch again an account already loaded (after a sync stored new mail). */
  reload: (accountId: string) => Promise<void>;
}

let nextRequestId = 0;

export const useSenderNameStore = create<SenderNameStore>((set, get) => {
  const load = async (accountId: string) => {
    nextRequestId += 1;
    const requestId = nextRequestId;
    set((state) => reduceRequested(state, accountId, requestId));
    try {
      const names = await api.getSenderNames(accountId);
      set((state) => reduceLoaded(state, accountId, requestId, names));
    } catch (error) {
      set((state) => reduceFailed(state, accountId, requestId));
      useLogStore.getState().addLog('error', 'system', `Could not load contact names: ${String(error)}`);
    }
  };

  return {
    ...INITIAL_SENDER_NAMES,
    ensureLoaded: async (accountId) => {
      const { namesByAccount, requests } = get();
      if (accountId in namesByAccount || accountId in requests) return;
      await load(accountId);
    },
    reload: async (accountId) => {
      if (!(accountId in get().namesByAccount)) return;
      await load(accountId);
    },
  };
});
