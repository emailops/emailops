// Auto-advance executor and its preference. The decision is the pure
// `planAdvance` (src/lib/autoAdvance.ts); this module captures what the user
// was looking at when an action started and acts on it once the action has
// taken the conversation out of the list.

import { create } from 'zustand';
import * as api from '@/lib/api';
import {
  AFTER_LEAVE_PREF,
  type AfterLeaveMode,
  DEFAULT_AFTER_LEAVE_MODE,
  parseAfterLeaveMode,
  planAdvance,
} from '@/lib/autoAdvance';
import { errorText } from '@/lib/errors';
import { selectionGeneration, threadKey, useEmailStore } from './emailStore';
import { useLogStore } from './logStore';
import { useShortcutStore } from './shortcutStore';

interface AutoAdvanceStore {
  mode: AfterLeaveMode;
  loadMode: () => Promise<void>;
  /** Persist the setting; throws (and keeps the old value) when it cannot. */
  setMode: (mode: AfterLeaveMode) => Promise<void>;
}

export const useAutoAdvanceStore = create<AutoAdvanceStore>((set) => ({
  mode: DEFAULT_AFTER_LEAVE_MODE,
  loadMode: async () => {
    try {
      set({ mode: parseAfterLeaveMode(await api.getPref(AFTER_LEAVE_PREF)) });
    } catch (err) {
      useLogStore.getState().addLog('error', 'system', `Could not read the auto-advance setting: ${errorText(err)}`);
    }
  },
  setMode: async (mode) => {
    await api.setPref(AFTER_LEAVE_PREF, mode);
    set({ mode });
  },
}));

/** What was on screen when a leaving action started. */
export interface LeaveTicket {
  /** `selectionGeneration()` then: any navigation since changes it. */
  generation: number;
  activeTabId: string | null;
  /** The open list row (main selection), if any. */
  emailId: string | null;
  /** The visible list, in order. */
  listIds: string[];
  /** Rows of the open conversation (they leave together). */
  leavingIds: Set<string>;
}

/** Call right before the action that takes the open conversation away. */
export function beginLeave(): LeaveTicket {
  const { selectedEmail, activeTabId } = useEmailStore.getState();
  const list = useShortcutStore.getState().listEmails;
  const key = selectedEmail ? threadKey(selectedEmail.accountId, selectedEmail.threadId) : null;
  return {
    generation: selectionGeneration(),
    activeTabId,
    emailId: selectedEmail?.id ?? null,
    listIds: list.map((e) => e.id),
    leavingIds: new Set(list.filter((e) => threadKey(e.accountId, e.threadId) === key).map((e) => e.id)),
  };
}

export interface FinishLeaveOptions {
  /** How this surface closes the conversation (default: clear the selection). */
  close?: () => void;
  /** Only move on when the open conversation actually left the list (a block
   *  started from elsewhere may concern another sender). */
  onlyIfLeft?: boolean;
}

export type LeaveOutcome = 'opened' | 'closed' | 'stale' | 'stayed';

/**
 * After the action: open the neighbour the setting asks for, or close. Does
 * nothing (`stale`) when the user navigated since `beginLeave` — a slow action
 * must never close or replace a conversation opened in the meantime.
 */
export function finishLeave(ticket: LeaveTicket, options: FinishLeaveOptions = {}): LeaveOutcome {
  const store = useEmailStore.getState();
  if (selectionGeneration() !== ticket.generation || store.activeTabId !== ticket.activeTabId) return 'stale';
  const present = new Set(store.emails.map((e) => e.id));
  if (options.onlyIfLeft) {
    // Nothing was open, or the open conversation is still in the list.
    if (ticket.emailId === null && ticket.activeTabId === null) return 'stayed';
    if (ticket.emailId !== null && present.has(ticket.emailId)) return 'stayed';
  }
  const close = options.close ?? (() => void useEmailStore.getState().selectEmail(null));

  const target =
    ticket.activeTabId === null && ticket.emailId !== null
      ? planAdvance(
          ticket.listIds,
          ticket.emailId,
          new Set([...ticket.leavingIds, ...ticket.listIds.filter((id) => !present.has(id))]),
          useAutoAdvanceStore.getState().mode,
        )
      : null;
  const email = target === null ? undefined : store.emails.find((e) => e.id === target);
  if (!email) {
    close();
    return 'closed';
  }
  store.setActiveTab(null);
  useShortcutStore.getState().setCursor(email.id);
  void store.selectEmail(email, undefined, { markRead: true });
  return 'opened';
}
