import { create } from 'zustand';
import { getAvailableUpdate, getPref, setPref } from '@/lib/api';
import {
  buildUpdateToast,
  parseUpdateDismissal,
  sanitizeAvailableUpdate,
  shouldShowUpdateToast,
  type UpdateAvailablePayload,
  type UpdateDismissal,
  type UpdateToastDeps,
} from '@/lib/appUpdate';
import type { Toast } from '@/stores/toastStore';

const PREF_DISMISSED_VERSION = 'app_update_dismissed_version';
const PREF_DISMISSED_AT = 'app_update_dismissed_at';

/** Where `remind` shows the toast: the toast store plus i18n and the
 *  external opener, injected so the store is testable without React/Tauri. */
export interface UpdateToastHost extends Omit<UpdateToastDeps, 'onDismiss'> {
  addToast: (toast: Omit<Toast, 'id'>) => number;
  isToastOpen: (id: number) => boolean;
}

/**
 * Latest-known newer release, backing the persistent download link in the
 * sidebar footer and the update toast. Populated at startup from the
 * `get_available_update` command (prefs persisted by the daily backend check)
 * and live from the `app-update-available` event. Both paths go through
 * `sanitizeAvailableUpdate`, so `available.url` is always safe to open.
 *
 * The toast is shown by `remind` (at startup, on the event and hourly) until
 * the user updates; closing it snoozes it for 24 hours (`dismissal`).
 */
interface UpdateStore {
  available: UpdateAvailablePayload | null;
  dismissal: UpdateDismissal | null;
  /** Id of the update toast while it is on screen. */
  toastId: number | null;
  setAvailable: (update: UpdateAvailablePayload | null) => void;
  load: () => Promise<void>;
  remind: (host: UpdateToastHost, nowSecs: number) => void;
  dismiss: (version: string, nowSecs: number) => Promise<void>;
}

export const useUpdateStore = create<UpdateStore>((set, get) => ({
  available: null,
  dismissal: null,
  toastId: null,
  setAvailable: (update) => set({ available: update }),
  load: async () => {
    try {
      const [update, dismissedVersion, dismissedAt] = await Promise.all([
        getAvailableUpdate(),
        getPref(PREF_DISMISSED_VERSION),
        getPref(PREF_DISMISSED_AT),
      ]);
      set({
        available: update ? sanitizeAvailableUpdate(update) : null,
        dismissal: parseUpdateDismissal(dismissedVersion, dismissedAt),
      });
    } catch (err) {
      // Purely informational — a missing update notice is better than an
      // error state (same stance as VersionLabel).
      console.error('Failed to load available update', err);
    }
  },
  remind: (host, nowSecs) => {
    const { available, dismissal, toastId } = get();
    if (toastId !== null && host.isToastOpen(toastId)) return;
    if (!available || !shouldShowUpdateToast(available, dismissal, nowSecs)) return;
    const id = host.addToast(
      buildUpdateToast(available, {
        t: host.t,
        openUrl: host.openUrl,
        onDismiss: () => void get().dismiss(available.version, Math.floor(Date.now() / 1000)),
      }),
    );
    set({ toastId: id });
  },
  dismiss: async (version, nowSecs) => {
    set({ dismissal: { version, at: nowSecs }, toastId: null });
    try {
      await setPref(PREF_DISMISSED_VERSION, version);
      await setPref(PREF_DISMISSED_AT, String(nowSecs));
    } catch (err) {
      // The snooze still holds for this session; only a restart within 24h
      // would show the toast early.
      console.error('Failed to persist update dismissal', err);
    }
  },
}));
