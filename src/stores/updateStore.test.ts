import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getAvailableUpdate: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { useUpdateStore } from './updateStore';

const RELEASE_URL = 'https://github.com/emailops/emailops/releases/tag/v0.7.0';

describe('updateStore', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useUpdateStore.setState({ available: null });
  });

  it('load() stores a validated available update from the backend', async () => {
    vi.mocked(api.getAvailableUpdate).mockResolvedValue({ version: '0.7.0', url: RELEASE_URL });
    await useUpdateStore.getState().load();
    expect(useUpdateStore.getState().available).toEqual({ version: '0.7.0', url: RELEASE_URL });
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

  it('load() swallows command failures (purely informational surface)', async () => {
    vi.mocked(api.getAvailableUpdate).mockRejectedValue(new Error('command failed'));
    await expect(useUpdateStore.getState().load()).resolves.toBeUndefined();
    expect(useUpdateStore.getState().available).toBeNull();
  });

  it('setAvailable replaces the current value', () => {
    useUpdateStore.getState().setAvailable({ version: '0.7.0', url: RELEASE_URL });
    expect(useUpdateStore.getState().available).toEqual({ version: '0.7.0', url: RELEASE_URL });
  });
});
