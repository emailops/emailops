import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';
import { parseShortcutsEnabled, SHORTCUTS_ENABLED_PREF, useShortcutStore } from './shortcutStore';

beforeEach(() => {
  useShortcutStore.setState({ enabled: true, paneCommand: null, bulkSnoozeRequested: false });
  vi.mocked(api.getPref).mockReset();
  vi.mocked(api.setPref).mockReset();
});

describe('parseShortcutsEnabled', () => {
  it('is on unless explicitly turned off', () => {
    expect(parseShortcutsEnabled(null)).toBe(true);
    expect(parseShortcutsEnabled('true')).toBe(true);
    expect(parseShortcutsEnabled('garbage')).toBe(true);
    expect(parseShortcutsEnabled('false')).toBe(false);
  });
});

describe('useShortcutStore', () => {
  it('loads the stored preference', async () => {
    vi.mocked(api.getPref).mockResolvedValue('false');
    await useShortcutStore.getState().loadEnabled();
    expect(api.getPref).toHaveBeenCalledWith(SHORTCUTS_ENABLED_PREF);
    expect(useShortcutStore.getState().enabled).toBe(false);
  });

  it('keeps shortcuts on when the preference cannot be read', async () => {
    vi.mocked(api.getPref).mockRejectedValue(new Error('db locked'));
    await useShortcutStore.getState().loadEnabled();
    expect(useShortcutStore.getState().enabled).toBe(true);
  });

  it('persists a change', async () => {
    await useShortcutStore.getState().setEnabled(false);
    expect(api.setPref).toHaveBeenCalledWith(SHORTCUTS_ENABLED_PREF, 'false');
    expect(useShortcutStore.getState().enabled).toBe(false);
  });

  it('keeps the old value when saving fails', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await expect(useShortcutStore.getState().setEnabled(false)).rejects.toThrow('disk full');
    expect(useShortcutStore.getState().enabled).toBe(true);
  });

  it('each pane command request is a new event, even when repeated', () => {
    useShortcutStore.getState().requestPaneCommand('reply');
    const first = useShortcutStore.getState().paneCommand;
    useShortcutStore.getState().requestPaneCommand('reply');
    const second = useShortcutStore.getState().paneCommand;
    expect(first?.command).toBe('reply');
    expect(second?.nonce).toBeGreaterThan(first?.nonce ?? 0);
  });

  it('a bulk snooze request waits until the toolbar consumes it', () => {
    useShortcutStore.getState().requestBulkSnooze();
    expect(useShortcutStore.getState().bulkSnoozeRequested).toBe(true);
    useShortcutStore.getState().consumeBulkSnooze();
    expect(useShortcutStore.getState().bulkSnoozeRequested).toBe(false);
  });
});
