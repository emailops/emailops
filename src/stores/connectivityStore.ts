import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { create } from 'zustand';
import * as api from '@/lib/api';

/**
 * Hybrid connectivity tracking.
 *
 * We combine two signals, but they are not equals:
 *   1. The backend probe (`app-connectivity-changed`, one per 15s) is
 *      **authoritative**. It performs a real HTTP request from this machine,
 *      so it catches captive portals, broken DNS and VPN drops that
 *      `navigator.onLine` lies about — and it is the same flag the Rust sync
 *      scheduler gates on, so the banner and the actual behaviour agree.
 *   2. The browser `offline` event is a **hint** that buys latency: it lets us
 *      show the banner the instant the link drops instead of up to 15s later.
 *
 * The hint may only pull us offline, never keep us there. WKWebView fires a
 * bare `offline` on transient link changes with no matching `online` to
 * follow, so anything that let the browser signal veto a later probe would
 * pin the banner forever — which is exactly the bug this shape fixes. Every
 * probe result overwrites the hint, so the worst case is one stale interval.
 *
 * Per CLAUDE.md, components must destructure reactive fields
 * (`const { isOnline } = useConnectivityStore()`); reading via
 * `useConnectivityStore.getState().isOnline` inside memo/effect deps will not
 * subscribe to updates.
 */
interface ConnectivityStore {
  /** Latest probe result, or `false` while the browser hint says otherwise. */
  isOnline: boolean;
  /** Latest backend probe result, kept so `online` can restore what it knew. */
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
        // The link is back, but only the probe knows whether the internet is.
        set((s) => ({ isOnline: s.backendOnline }));
      };
      const onOffline = () => {
        // Show the banner now rather than up to a probe interval later. If the
        // event was spurious, the next probe undoes this.
        set({ isOnline: false });
      };
      window.addEventListener('online', onOnline);
      window.addEventListener('offline', onOffline);

      // One event per probe, so this always re-converges on the truth.
      const unlistenBackend = await listen<{ online: boolean }>('app-connectivity-changed', (event) => {
        const backendOnline = event.payload.online;
        set({ backendOnline, isOnline: backendOnline });
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
        set({ backendOnline, isOnline: backendOnline, initialized: true });
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
