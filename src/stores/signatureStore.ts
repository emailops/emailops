import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { AccountSignature, SignatureInput } from '@/types';
import { useLogStore } from './logStore';

/**
 * Account signatures, loaded on demand and cached for the composers
 * (`useComposerSignature`). The settings editor saves through here so an open
 * composer's next lookup sees the new signature.
 */
interface SignatureStore {
  byAccount: Record<string, AccountSignature>;
  /** The account's signature, from the cache or the backend; null when it could not be loaded (logged). */
  load: (accountId: string) => Promise<AccountSignature | null>;
  /** Save and cache what the backend stored (sanitized). Rejects on failure. */
  save: (accountId: string, input: SignatureInput) => Promise<AccountSignature>;
}

/** Pure: the cache with `signature` in it. */
export function withSignature(
  byAccount: Record<string, AccountSignature>,
  signature: AccountSignature,
): Record<string, AccountSignature> {
  return { ...byAccount, [signature.accountId]: signature };
}

export const useSignatureStore = create<SignatureStore>((set, get) => ({
  byAccount: {},

  load: async (accountId) => {
    const cached = get().byAccount[accountId];
    if (cached) return cached;
    try {
      const signature = await api.getAccountSignature(accountId);
      set((state) => ({ byAccount: withSignature(state.byAccount, signature) }));
      return signature;
    } catch (err) {
      useLogStore.getState().addLog('error', 'account', `Could not load the signature: ${errorText(err)}`);
      return null;
    }
  },

  save: async (accountId, input) => {
    const saved = await api.saveAccountSignature(accountId, input);
    set((state) => ({ byAccount: withSignature(state.byAccount, saved) }));
    return saved;
  },
}));
