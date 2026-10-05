import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getAvailableUpdate: vi.fn(),
  getPref: vi.fn(),
  setPref: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { UPDATE_REMINDER_INTERVAL_SECS } from '@/lib/appUpdate';
import { type UpdateToastHost, useUpdateStore } from './updateStore';

const RELEASE_URL = 'https://github.com/emailops/emailops/releases/tag/v0.7.0';
const UPDATE = { version: '0.7.0', url: RELEASE_URL };
const NOW = 1_000_000;

/** In-memory toast host: records shown toasts and which ids are still open. */
function makeHost() {
  const open = new Set<number>();
  const shown: Parameters<UpdateToastHost['addToast']>[0][] = [];
  const host: UpdateToastHost = {
    addToast: (toast) => {
      shown.push(toast);
      const id = shown.length;
      open.add(id);
      return id;
    },
    isToastOpen: (id) => open.has(id),
    t: (key) => key,
    openUrl: vi.fn(),
  };
  return { host, shown, open };
}

describe('updateStore', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(api.getPref).mockResolvedValue(null);
    vi.mocked(api.setPref).mockResolvedValue(undefined);
    useUpdateStore.setState({ available: null, dismissal: null, toastId: null });
  });

  it('load() stores a validated available update from the backend', async () => {
    vi.mocked(api.getAvailableUpdate).mockResolvedValue(UPDATE);
    await useUpdateStore.getState().load();
    expect(useUpdateStore.getState().available).toEqual(UPDATE);
  });

  it('load() leaves null when the backend reports no update', async () => {
    vi.mocked(api.getAvailableUpdate).mockResolvedValue(null);
    await useUpdateStore.getState().load();
    expect(useUpdateStore.getState().available).toBeNull();
  });

  it('load() drops updates whose url is not a github release page', async () => {
    vi.mocked(api.getAvailableUpdate).mockResolvedValue({ version: '0.7.0', url: 'https://evil.com/x' });
    await useUpdateStore.getState().load();
    expect(useUpdateStore.getState().available).toBeNull();
  });

  it('load() reads the persisted dismissal', async () => {
    vi.mocked(api.getAvailableUpdate).mockResolvedValue(UPDATE);
    vi.mocked(api.getPref).mockImplementation(async (key) =>
      key === 'app_update_dismissed_version' ? '0.7.0' : key === 'app_update_dismissed_at' ? String(NOW) : null,
    );
    await useUpdateStore.getState().load();
    expect(useUpdateStore.getState().dismissal).toEqual({ version: '0.7.0', at: NOW });
  });

  it('load() swallows command failures (purely informational surface)', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    vi.mocked(api.getAvailableUpdate).mockRejectedValue(new Error('command failed'));
    await expect(useUpdateStore.getState().load()).resolves.toBeUndefined();
    expect(useUpdateStore.getState().available).toBeNull();
  });

  it('setAvailable replaces the current value', () => {
    useUpdateStore.getState().setAvailable(UPDATE);
    expect(useUpdateStore.getState().available).toEqual(UPDATE);
  });

  it('remind() shows the toast for an available update', () => {
    const { host, shown } = makeHost();
    useUpdateStore.setState({ available: UPDATE });
    useUpdateStore.getState().remind(host, NOW);
    expect(shown).toHaveLength(1);
  });

  it('remind() does not stack a second toast while one is open', () => {
    const { host, shown } = makeHost();
    useUpdateStore.setState({ available: UPDATE });
    useUpdateStore.getState().remind(host, NOW);
    useUpdateStore.getState().remind(host, NOW + 3_600);
    expect(shown).toHaveLength(1);
  });

  it('closing the toast snoozes it for 24 hours and persists the dismissal', async () => {
    const { host, shown, open } = makeHost();
    useUpdateStore.setState({ available: UPDATE });
    useUpdateStore.getState().remind(host, NOW);

    open.delete(1);
    await shown[0]?.onDismiss?.();

    expect(api.setPref).toHaveBeenCalledWith('app_update_dismissed_version', '0.7.0');
    expect(api.setPref).toHaveBeenCalledWith('app_update_dismissed_at', expect.stringMatching(/^\d+$/));
    const dismissedAt = useUpdateStore.getState().dismissal?.at ?? 0;

    useUpdateStore.getState().remind(host, dismissedAt + UPDATE_REMINDER_INTERVAL_SECS - 1);
    expect(shown).toHaveLength(1);

    useUpdateStore.getState().remind(host, dismissedAt + UPDATE_REMINDER_INTERVAL_SECS);
    expect(shown).toHaveLength(2);
  });

  it('remind() shows nothing once the user has updated', () => {
    const { host, shown } = makeHost();
    useUpdateStore.setState({ available: null });
    useUpdateStore.getState().remind(host, NOW);
    expect(shown).toHaveLength(0);
  });
});
