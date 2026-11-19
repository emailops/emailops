// The archive/delete undo window preference, stored in SQLite like Undo send.

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';
import { UNDO_WINDOW_PREF } from '@/lib/undoWindow';
import { useLogStore } from './logStore';
import { undoWindowMs, useUndoWindowStore } from './undoWindowStore';

beforeEach(() => {
  useUndoWindowStore.setState({ secs: 6 });
  vi.clearAllMocks();
});

describe('the undo window preference', () => {
  it('is 6 s until loaded', () => {
    expect(undoWindowMs()).toBe(6000);
  });

  it('loads from SQLite, defaulting to 6 s', async () => {
    vi.mocked(api.getPref).mockResolvedValueOnce('20');
    await useUndoWindowStore.getState().load();
    expect(api.getPref).toHaveBeenCalledWith(UNDO_WINDOW_PREF);
    expect(undoWindowMs()).toBe(20_000);
    vi.mocked(api.getPref).mockResolvedValueOnce(null);
    await useUndoWindowStore.getState().load();
    expect(undoWindowMs()).toBe(6000);
  });

  it('keeps the default and logs when the preference cannot be read', async () => {
    vi.mocked(api.getPref).mockRejectedValueOnce(new Error('db locked'));
    await useUndoWindowStore.getState().load();
    expect(undoWindowMs()).toBe(6000);
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('db locked'))).toBe(
      true,
    );
  });

  it('saves, and keeps the old value when saving fails', async () => {
    await useUndoWindowStore.getState().setSecs(0);
    expect(api.setPref).toHaveBeenCalledWith(UNDO_WINDOW_PREF, '0');
    expect(undoWindowMs()).toBe(0);
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('disk full'));
    await expect(useUndoWindowStore.getState().setSecs(30)).rejects.toThrow('disk full');
    expect(undoWindowMs()).toBe(0);
  });
});
