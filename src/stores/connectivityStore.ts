import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { create } from 'zustand';
import * as api from '@/lib/api';

/**
 * Hybrid connectivity tracking.
 *
 * We combine two signals:
 *   1. `navigator.onLine` (instant — fires on link-layer changes like
 *      unplugging Wi-Fi)
 *   2. Backend probe to a neutral host every 15s (authoritative — catches
 *      captive portals, broken DNS, VPN drops where `navigator.onLine` lies)
 *
 * `isOnline` is the AND of both signals: any "I'm definitely offline" signal
 * wins. This biases toward showing the offline banner sooner rather than
 * later, which is what we want — a stale "online" indicator while sync
 * silently fails is worse than a brief false-positive banner.
 *
 * Per CLAUDE.md, components must destructure reactive fields
 * (`const { isOnline } = useConnectivityStore()`); reading via
 * `useConnectivityStore.getState().isOnline` inside memo/effect deps will not
 * subscribe to updates.
 */
interface ConnectivityStore {
  /** Combined signal: navigator says online AND last probe succeeded. */
  isOnline: boolean;
  /** Latest browser signal, exposed for debugging / advanced UIs. */
  navigatorOnline: boolean;
  /** Latest backend probe result. */
  backendOnline: boolean;
  /** Whether the initial probe has resolved. Until then, default to online so
   *  we don't flash an "Offline" banner during normal startup. */
  initialized: boolean;
  /** Subscribe to native events. Idempotent — calling twice is a no-op. */
  init: () => Promise<void>;
}

let unlistenFn: UnlistenFn | null = null;
let initInFlight: Promise<void> | null = null;

export const useConnectivityStore = create<ConnectivityStore>((set) => ({
  isOnline: true,
  navigatorOnline: typeof navigator === 'undefined' ? true : navigator.onLine,
  backendOnline: true,
  initialized: false,

  init: async () => {
    // Already running (StrictMode double-invoke or multiple mount points)?
    // Share the in-flight promise so callers all await the same setup.
    if (initInFlight) return initInFlight;
    if (unlistenFn) return;

    initInFlight = (async () => {
      // Browser events — cheap to attach and respond instantly to OS-level
      // network changes.
      const onOnline = () => {
        set((s) => ({ navigatorOnline: true, isOnline: s.backendOnline }));
      };
      const onOffline = () => {
        // navigator says offline — trust it immediately. The next probe will
        // confirm and flip backendOnline if needed.
        set({ navigatorOnline: false, isOnline: false });
      };
      window.addEventListener('online', onOnline);
      window.addEventListener('offline', onOffline);

      // Backend probe events — transitions only (the backend already de-dupes).
      const unlistenBackend = await listen<{ online: boolean }>('app-connectivity-changed', (event) => {
        const backendOnline = event.payload.online;
        set((s) => ({ backendOnline, isOnline: s.navigatorOnline && backendOnline }));
      });

      unlistenFn = () => {
        window.removeEventListener('online', onOnline);
        window.removeEventListener('offline', onOffline);
        unlistenBackend();
      };

      // Seed with the backend's cached state so we don't wait up to 15s for
      // the first event if startup happened while offline.
      try {
        const backendOnline = await api.isOnline();
        set((s) => ({
          backendOnline,
          isOnline: s.navigatorOnline && backendOnline,
          initialized: true,
        }));
      } catch {
        // Backend probe failed to respond — treat as initialized but defer
        // to whatever signal we have. We don't flip `isOnline` to false here
        // because a flaky IPC call shouldn't be confused with no internet.
        set({ initialized: true });
      }
    })();

    try {
      await initInFlight;
    } finally {
      initInFlight = null;
    }
  },
}));
