// A flagged row must never paint without its warning: the list waits for the
// verdicts before it publishes a page, in one place, for every kind of list.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Email, JunkVerdict } from '@/types';

vi.mock('@/lib/api', () => ({
  getEmails: vi.fn(async () => []),
  getEmailCount: vi.fn(async () => 0),
  listThreadSnoozes: vi.fn(async () => []),
  getJunkVerdicts: vi.fn(async () => ({})),
}));

import * as api from '@/lib/api';
import { useEmailStore } from './emailStore';
import { useJunkStore } from './junkStore';

function row(id: string): Email {
  return { id, accountId: 'acc', threadId: `t-${id}`, isRead: false, timestamp: 1, subject: id } as Email;
}

const VERDICT = { emailId: 'e1', band: 'junk', primaryKind: 'phishing', userOverride: null } as JunkVerdict;

describe('rows wait for their verdicts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useJunkStore.setState({ verdictsByEmail: {}, loaded: {} });
    useEmailStore.setState({ searchQuery: null, emails: [], totalCount: 0, error: null });
  });

  it('holds a first page until its verdicts are in', async () => {
    vi.mocked(api.getEmails).mockResolvedValue([row('e1')]);
    vi.mocked(api.getEmailCount).mockResolvedValue(1);
    let release: (verdicts: Record<string, JunkVerdict>) => void = () => {};
    vi.mocked(api.getJunkVerdicts).mockReturnValue(
      new Promise((resolve) => {
        release = resolve;
      }),
    );

    const fetching = useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');
    await vi.waitFor(() => expect(api.getJunkVerdicts).toHaveBeenCalledWith(['e1']));
    expect(useEmailStore.getState().emails).toEqual([]);

    release({ e1: VERDICT });
    await fetching;

    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['e1']);
    expect(useJunkStore.getState().verdictsByEmail.e1).toBe(VERDICT);
  });

  it('a first page without verdicts is a failed page, not rows without a warning', async () => {
    vi.mocked(api.getEmails).mockResolvedValue([row('e1')]);
    vi.mocked(api.getEmailCount).mockResolvedValue(1);
    vi.mocked(api.getJunkVerdicts).mockRejectedValue(new Error('no verdicts'));

    await useEmailStore.getState().fetchEmails('acc', null, [], false, 'inbox');

    expect(useEmailStore.getState().emails).toEqual([]);
    expect(useEmailStore.getState().error).toContain('no verdicts');
  });

  it('holds the next page too', async () => {
    useEmailStore.setState({ emails: [row('e0')], paging: { status: 'idle', cursor: null, offset: 1 }, totalCount: 2 });
    vi.mocked(api.getEmails).mockResolvedValue([row('e1')]);
    vi.mocked(api.getJunkVerdicts).mockRejectedValue(new Error('no verdicts'));

    await useEmailStore.getState().loadMoreEmails('acc', null, [], 'inbox');

    expect(useEmailStore.getState().emails.map((e) => e.id)).toEqual(['e0']);
  });
});
