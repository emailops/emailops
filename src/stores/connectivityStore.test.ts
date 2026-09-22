// Regression: the offline banner latched on forever while sync kept working.
//
// `isOnline` used to be `navigatorOnline && backendOnline`, where
// `navigatorOnline` was seeded once and thereafter only moved by the window
// `online` / `offline` events. WKWebView fires a bare `offline` on transient
// link changes (Wi-Fi roam, VPN toggle, an interface flapping under a heavy
// sync) with no matching `online` to follow, because the OS never actually
// went down. That pinned `navigatorOnline` to false with no way back, and the
// backend probe — the authoritative signal, the same flag the Rust sync
// scheduler trusts — could never out-vote it: `false && true` is still false.
//
// The backend probe is now authoritative. A browser `offline` event stays a
// fast pessimistic hint so pulling the cable still shows the banner at once,
// but the next probe result always wins. These tests pin both halves.

import { beforeEach, describe, expect, it, vi } from 'vitest';

type BackendHandler = (event: { payload: { online: boolean } }) => void;

let backendHandler: BackendHandler | null = null;
const isOnlineMock = vi.fn(() => Promise.resolve(true));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((_name: string, handler: BackendHandler) => {
    backendHandler = handler;
    return Promise.resolve(() => {});
  }),
}));

vi.mock('@/lib/api', () => ({
  isOnline: () => isOnlineMock(),
}));

/** Fresh module instance per test — the store keeps init state at module scope. */
async function freshStore() {
  vi.resetModules();
  backendHandler = null;
  const { useConnectivityStore } = await import('./connectivityStore');
  await useConnectivityStore.getState().init();
  return useConnectivityStore;
}

function backendReports(online: boolean) {
  if (!backendHandler) throw new Error('store never subscribed to app-connectivity-changed');
  backendHandler({ payload: { online } });
}

describe('connectivityStore', () => {
  beforeEach(() => {
    isOnlineMock.mockReset();
    isOnlineMock.mockResolvedValue(true);
  });

  it('recovers from a spurious offline event once the backend probe reports again', async () => {
    const store = await freshStore();
    expect(store.getState().isOnline).toBe(true);

    // WKWebView fires `offline` although the machine never lost its link.
    window.dispatchEvent(new Event('offline'));
    expect(store.getState().isOnline).toBe(false);

    // The next probe finds the network perfectly healthy — and must be believed.
    backendReports(true);
    expect(store.getState().isOnline).toBe(true);
  });

  it('shows offline immediately on the browser event, without waiting for a probe', async () => {
    const store = await freshStore();

    window.dispatchEvent(new Event('offline'));

    expect(store.getState().isOnline).toBe(false);
  });

  it('stays offline through a real outage and clears when the probe recovers', async () => {
    const store = await freshStore();

    window.dispatchEvent(new Event('offline'));
    backendReports(false);
    expect(store.getState().isOnline).toBe(false);

    // A browser `online` event alone is not proof; the probe still says no.
    window.dispatchEvent(new Event('online'));
    expect(store.getState().isOnline).toBe(false);

    backendReports(true);
    expect(store.getState().isOnline).toBe(true);
  });

  it('seeds from the backend so a cold start while offline shows the banner', async () => {
    isOnlineMock.mockResolvedValue(false);

    const store = await freshStore();

    expect(store.getState().isOnline).toBe(false);
    expect(store.getState().initialized).toBe(true);
  });

  it('marks itself initialized even when the seed IPC call fails', async () => {
    isOnlineMock.mockRejectedValue(new Error('ipc unavailable'));

    const store = await freshStore();

    expect(store.getState().initialized).toBe(true);
    expect(store.getState().isOnline).toBe(true);
  });
});
