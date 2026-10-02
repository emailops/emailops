import { create } from 'zustand';
import { i18n } from '@/i18n';
import type { OutboxEntry, OutboxUpdated, OutgoingMessage } from '@/lib/api';
import * as api from '@/lib/api';
import { errorText, isAppErrorPayload } from '@/lib/errors';
import { formatDateTime } from '@/lib/intl';
import { parseUndoSendDelay, UNDO_SEND_DELAY_PREF } from '@/lib/outbox';
import { useLogStore } from '@/stores/logStore';
import { useToastStore } from '@/stores/toastStore';

/**
 * Undo send and scheduled send (the local outbox).
 *
 * `send` is the single Send entry point of every composer: with undo send off
 * it runs the composer's direct send; otherwise the message is queued for the
 * undo window and a "Sending… Undo" toast stays up exactly that long. Undo
 * asks the backend to take the message back — the backend decides, so an Undo
 * that loses the race against the dispatcher reports "already sent" instead of
 * reopening a message that went out.
 *
 * Reopening a composer and showing the Scheduled view belong to the app shell,
 * which registers them with `setRestoreHandler` / `setShowScheduledHandler`.
 */

type RestoreHandler = (message: OutgoingMessage) => void;

/** An outbox row as the backend names it: the account it belongs to and its id. */
type OutboxRef = Pick<OutboxEntry, 'id' | 'accountId'>;

interface OutboxStore {
  /** Waiting and failed rows of `scope` (the Scheduled view). */
  entries: OutboxEntry[];
  /** Account the entries were listed for (`null` = every enabled account). */
  scope: string | null;
  /** The undo window in seconds, once read from the preferences. */
  undoDelaySecs: number | null;
  loadUndoDelay: () => Promise<number>;
  setUndoDelay: (secs: number) => Promise<void>;
  fetchEntries: (accountId?: string | null) => Promise<void>;
  /** Send from a composer: 'sent' when it went out directly, 'queued' when it
   *  waits for the undo window. Throws what the direct send or the queueing
   *  threw, so the composer keeps the text and shows the error. */
  send: (
    message: OutgoingMessage,
    opts: { draftId?: string; sendDirect: () => Promise<void> },
  ) => Promise<'sent' | 'queued'>;
  /** Schedule a message for `at`. Throws when it cannot be queued. */
  schedule: (message: OutgoingMessage, at: Date, draftId?: string) => Promise<OutboxEntry>;
  /** Scheduled view: cancel and reopen the composer with the message. */
  edit: (entry: OutboxRef) => Promise<void>;
  /** Scheduled view: delete the message (its toast's Undo reopens it). */
  remove: (entry: OutboxRef) => Promise<void>;
  /** Scheduled view: send now / retry. */
  sendNow: (entry: OutboxRef) => Promise<void>;
  /** Apply an `outbox-updated` event from the dispatcher. */
  applyUpdate: (update: OutboxUpdated) => void;
  setRestoreHandler: (handler: RestoreHandler | null) => void;
  setShowScheduledHandler: (handler: (() => void) | null) => void;
}

/** Pure: `entries` without the row `id`. */
export function withoutEntry(entries: OutboxEntry[], id: string): OutboxEntry[] {
  return entries.filter((e) => e.id !== id);
}

let restoreHandler: RestoreHandler | null = null;
let showScheduledHandler: (() => void) | null = null;

const toasts = () => useToastStore.getState();
const log = (level: 'info' | 'success' | 'error', message: string) =>
  useLogStore.getState().addLog(level, 'sync', message);

function restore(message: OutgoingMessage) {
  if (restoreHandler) restoreHandler(message);
  else log('error', 'No composer is available to reopen the message');
}

const isNotPending = (err: unknown) => isAppErrorPayload(err) && err.code === 'outbox_not_pending';

/** Take a queued message back. Returns it, or null when it is too late (the
 *  user is told) or failed (logged and shown). */
async function takeBack(entry: OutboxRef): Promise<OutgoingMessage | null> {
  try {
    return await api.cancelOutboxMessage(entry.accountId, entry.id);
  } catch (err) {
    const text = isNotPending(err) ? i18n.t('compose:outbox.tooLate') : errorText(err);
    toasts().addToast({ message: text });
    log('error', `Could not take the message back: ${errorText(err)}`);
    return null;
  }
}

const fmtTime = (at: Date) =>
  formatDateTime(Math.floor(at.getTime() / 1000), i18n.language || 'en', {
    weekday: 'short',
    day: 'numeric',
    month: 'short',
    hour: '2-digit',
    minute: '2-digit',
  });

export const useOutboxStore = create<OutboxStore>((set, get) => ({
  entries: [],
  scope: null,
  undoDelaySecs: null,

  loadUndoDelay: async () => {
    const cached = get().undoDelaySecs;
    if (cached !== null) return cached;
    let secs: number;
    try {
      secs = parseUndoSendDelay(await api.getPref(UNDO_SEND_DELAY_PREF));
    } catch (err) {
      log('error', `Could not read the undo-send setting: ${errorText(err)}`);
      secs = parseUndoSendDelay(null);
    }
    set({ undoDelaySecs: secs });
    return secs;
  },

  setUndoDelay: async (secs) => {
    await api.setPref(UNDO_SEND_DELAY_PREF, String(secs));
    set({ undoDelaySecs: secs });
  },

  fetchEntries: async (accountId) => {
    if (accountId !== undefined) set({ scope: accountId });
    const scope = get().scope;
    try {
      const entries = await api.listOutbox(scope);
      // A fetch for another scope may have started meanwhile; it wins.
      if (get().scope === scope) set({ entries });
    } catch (err) {
      log('error', `Could not list scheduled messages: ${errorText(err)}`);
    }
  },

  send: async (message, { draftId, sendDirect }) => {
    const delay = await get().loadUndoDelay();
    if (delay === 0) {
      await sendDirect();
      return 'sent';
    }
    const queued = await api.queueOutgoingEmail(message, { type: 'undo', delaySecs: delay }, draftId);
    log('info', `Sending to ${message.to.join(', ')} in ${delay}s (undo available)`);
    let toastId = 0;
    toastId = toasts().addToast({
      message: i18n.t('compose:outbox.sending'),
      actionLabel: i18n.t('compose:outbox.undo'),
      durationMs: delay * 1000,
      onAction: () => {
        toasts().dismissToast(toastId);
        void takeBack(queued).then((taken) => {
          if (!taken) return;
          log('info', 'Send undone');
          restore(taken);
        });
      },
    });
    return 'queued';
  },

  schedule: async (message, at, draftId) => {
    const sendAt = Math.floor(at.getTime() / 1000);
    const queued = await api.queueOutgoingEmail(message, { type: 'at', sendAt }, draftId);
    log('success', `Message scheduled for ${at.toISOString()}`);
    toasts().addToast({
      message: i18n.t('compose:outbox.scheduledFor', { time: fmtTime(at) }),
      actionLabel: i18n.t('compose:outbox.undo'),
      onAction: () => {
        void takeBack(queued).then((taken) => {
          if (taken) restore(taken);
          void get().fetchEntries();
        });
      },
    });
    void get().fetchEntries();
    return queued;
  },

  edit: async (entry) => {
    const taken = await takeBack(entry);
    set((s) => ({ entries: withoutEntry(s.entries, entry.id) }));
    if (taken) restore(taken);
    else void get().fetchEntries();
  },

  remove: async (entry) => {
    const taken = await takeBack(entry);
    set((s) => ({ entries: withoutEntry(s.entries, entry.id) }));
    if (!taken) {
      void get().fetchEntries();
      return;
    }
    log('info', 'Scheduled message deleted');
    // The inverse of a delete: reopen the message in a composer.
    toasts().addToast({
      message: i18n.t('compose:outbox.deleted'),
      actionLabel: i18n.t('compose:outbox.undo'),
      onAction: () => restore(taken),
    });
  },

  sendNow: async (entry) => {
    try {
      await api.sendOutboxMessageNow(entry.accountId, entry.id);
      log('info', 'Sending a scheduled message now');
    } catch (err) {
      toasts().addToast({ message: isNotPending(err) ? i18n.t('compose:outbox.tooLate') : errorText(err) });
      log('error', `Could not send the message now: ${errorText(err)}`);
    }
    void get().fetchEntries();
  },

  applyUpdate: (update) => {
    const sentIds = new Set(update.sent.map((s) => s.id));
    if (sentIds.size > 0) {
      set((s) => ({ entries: s.entries.filter((e) => !sentIds.has(e.id)) }));
      log('success', i18n.t('compose:outbox.sentCount', { count: sentIds.size }));
    }
    for (const failure of update.failed) {
      const message = failure.interrupted
        ? i18n.t('compose:outbox.interrupted')
        : i18n.t('compose:outbox.failed', { error: failure.message });
      log('error', message);
      toasts().addToast({
        message,
        sticky: true,
        actionLabel: showScheduledHandler ? i18n.t('compose:outbox.view') : undefined,
        onAction: showScheduledHandler ?? undefined,
      });
    }
    if (update.failed.length > 0) void get().fetchEntries();
  },

  setRestoreHandler: (handler) => {
    restoreHandler = handler;
  },

  setShowScheduledHandler: (handler) => {
    showScheduledHandler = handler;
  },
}));
