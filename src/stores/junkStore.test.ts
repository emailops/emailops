// Regression: "Not junk" reported success it never had.
//
// `setFeedback` writes optimistically — it clears the verdict's override and
// strips the junk chip so the correction feels instant — and then awaited
// `api.setJunkFeedback` with no handler at all. The only caller (`JunkBanner`'s
// "Not junk" button) invoked it as `void setFeedback(...)`, so a failed write
// became an unhandled rejection: the banner vanished, the row un-faded, and the
// message came back flagged on the next reload with nothing ever having told
// the user why.
//
// The store now restores exactly what it overwrote and re-throws, so the caller
// can surface the failure. These tests pin both halves.

import { beforeEach, describe, expect, it, vi } from 'vitest';

const setJunkFeedback = vi.fn(() => Promise.resolve());

vi.mock('@/lib/api', () => ({
  setJunkFeedback: (...args: unknown[]) => setJunkFeedback(...(args as [])),
  getJunkConfig: vi.fn(() => Promise.resolve({ enabled: true, phishingEnabled: false, flaggedAction: 'dim' })),
  setJunkConfig: vi.fn(() => Promise.resolve()),
  getJunkVerdicts: vi.fn(() => Promise.resolve({})),
  currentPlatform: vi.fn(() => ''),
}));

import type { JunkVerdict } from '@/types';
import { isFlagged, useJunkStore } from './junkStore';
import { useTagStore } from './tagStore';

const FLAGGED: JunkVerdict = {
  emailId: 'e1',
  spamScore: 0.9,
  phishScore: 0,
  grayScore: 0,
  band: 'junk',
  primaryKind: 'spam',
  reasons: [],
  method: 'deterministic',
  modelVersion: 1,
  scoredAt: 0,
  userOverride: null,
};

const JUNK_CHIP = {
  emailId: 'e1',
  tagType: 'junk',
  tagValue: 'spam',
  confidence: null,
  createdAt: 0,
};

function seed(): void {
  useJunkStore.setState({ verdictsByEmail: { e1: FLAGGED }, loaded: { e1: true } });
  useTagStore.setState({ tagsByEmail: { e1: [JUNK_CHIP] } });
}

describe('junkStore.setFeedback', () => {
  beforeEach(() => {
    setJunkFeedback.mockReset();
    setJunkFeedback.mockResolvedValue(undefined);
    seed();
  });

  it('clears the badge and the chip once the write lands', async () => {
    await useJunkStore.getState().setFeedback('a1', 'e1', false);

    expect(isFlagged(useJunkStore.getState().verdictsByEmail.e1)).toBe(false);
    expect(useTagStore.getState().tagsByEmail.e1).toEqual([]);
    expect(setJunkFeedback).toHaveBeenCalledWith('a1', 'e1', false);
  });

  it('puts the badge and the chip back when the write fails, and re-throws', async () => {
    setJunkFeedback.mockRejectedValue(new Error('database is locked'));

    await expect(useJunkStore.getState().setFeedback('a1', 'e1', false)).rejects.toThrow('database is locked');

    // The user disagreed and the app failed to record it. Showing the message
    // as cleared would be a lie that survives until the next reload.
    expect(isFlagged(useJunkStore.getState().verdictsByEmail.e1)).toBe(true);
    expect(useTagStore.getState().tagsByEmail.e1).toEqual([JUNK_CHIP]);
  });

  it('rolls back a failed "confirm junk" the same way', async () => {
    useJunkStore.setState({ verdictsByEmail: { e1: { ...FLAGGED, band: 'clean' } } });
    useTagStore.setState({ tagsByEmail: { e1: [] } });
    setJunkFeedback.mockRejectedValue(new Error('offline'));

    await expect(useJunkStore.getState().setFeedback('a1', 'e1', true)).rejects.toThrow('offline');

    expect(useJunkStore.getState().verdictsByEmail.e1?.userOverride).toBeNull();
    expect(useTagStore.getState().tagsByEmail.e1).toEqual([]);
  });
});
