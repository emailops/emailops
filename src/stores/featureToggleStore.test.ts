import { beforeEach, describe, expect, it, vi } from 'vitest';

const getPref = vi.fn();
vi.mock('@/lib/api', () => ({ getPref: (key: string) => getPref(key), setPref: vi.fn() }));

import { useLensesEnabledStore, useSharedDocsEnabledStore } from './featureToggleStore';

describe('useLensesEnabledStore', () => {
  beforeEach(() => getPref.mockReset());

  it('is on when the user never set the preference', async () => {
    getPref.mockResolvedValue(null);
    await useLensesEnabledStore.getState().refresh();
    expect(useLensesEnabledStore.getState().enabled).toBe(true);
  });

  it('stays off when the user turned it off', async () => {
    getPref.mockResolvedValue('false');
    await useLensesEnabledStore.getState().refresh();
    expect(useLensesEnabledStore.getState().enabled).toBe(false);
  });
});

describe('useSharedDocsEnabledStore', () => {
  beforeEach(() => getPref.mockReset());

  it('is on when the user never set the preference', async () => {
    getPref.mockResolvedValue(null);
    await useSharedDocsEnabledStore.getState().refresh();
    expect(useSharedDocsEnabledStore.getState().enabled).toBe(true);
  });

  it('stays off when the user turned it off', async () => {
    getPref.mockResolvedValue('false');
    await useSharedDocsEnabledStore.getState().refresh();
    expect(useSharedDocsEnabledStore.getState().enabled).toBe(false);
  });
});
