// The account new blank messages start from (Settings → Appearance). The
// decision is the pure `newMessageAccountId` (src/lib/composeSender.ts).

import { create } from 'zustand';
import * as api from '@/lib/api';
import { DEFAULT_SENDER_PREF, parseDefaultSender } from '@/lib/composeSender';
import { errorText } from '@/lib/errors';
import { useLogStore } from './logStore';

interface DefaultSenderStore {
  /** `null`: the account of the selected view. */
  accountId: string | null;
  load: () => Promise<void>;
  /** Persist the setting; throws (and keeps the old value) when it cannot. */
  setAccountId: (accountId: string | null) => Promise<void>;
}

export const useDefaultSenderStore = create<DefaultSenderStore>((set) => ({
  accountId: null,
  load: async () => {
    try {
      set({ accountId: parseDefaultSender(await api.getPref(DEFAULT_SENDER_PREF)) });
    } catch (err) {
      useLogStore.getState().addLog('error', 'system', `Could not read the default sender: ${errorText(err)}`);
    }
  },
  setAccountId: async (accountId) => {
    await api.setPref(DEFAULT_SENDER_PREF, accountId ?? '');
    set({ accountId });
  },
}));
