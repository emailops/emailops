// The conversation view setting (Settings → Appearance), read by `EmailView`.
// The layout rules are in `src/lib/conversationView.ts`.

import { create } from 'zustand';
import * as api from '@/lib/api';
import {
  CONVERSATION_VIEW_PREF,
  type ConversationView,
  DEFAULT_CONVERSATION_VIEW,
  parseConversationView,
} from '@/lib/conversationView';
import { errorText } from '@/lib/errors';
import { useLogStore } from './logStore';

interface ConversationViewStore {
  /** The default until `load` resolves. */
  view: ConversationView;
  load: () => Promise<void>;
  /** Persist the setting; throws (and keeps the old value) when it cannot. */
  setView: (view: ConversationView) => Promise<void>;
}

export const useConversationViewStore = create<ConversationViewStore>((set) => ({
  view: DEFAULT_CONVERSATION_VIEW,
  load: async () => {
    try {
      set({ view: parseConversationView(await api.getPref(CONVERSATION_VIEW_PREF)) });
    } catch (err) {
      useLogStore
        .getState()
        .addLog('error', 'system', `Could not read the conversation view setting: ${errorText(err)}`);
    }
  },
  setView: async (view) => {
    await api.setPref(CONVERSATION_VIEW_PREF, view);
    set({ view });
  },
}));
