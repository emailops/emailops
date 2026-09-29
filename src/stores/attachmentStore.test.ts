// Unit tests for the attachment-rule suggestion slice of attachmentStore:
// async actions with the api layer mocked, plus the count selector.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AttachmentRuleSuggestion } from '@/types';
import { selectSuggestionCount, useAttachmentStore } from './attachmentStore';

vi.mock('@/lib/api', () => ({
  listAttachmentRuleSuggestions: vi.fn(async () => []),
  refreshAttachmentRuleSuggestions: vi.fn(async () => []),
  dismissAttachmentRuleSuggestion: vi.fn(async () => {}),
  acceptAttachmentRuleSuggestion: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';

function makeSuggestion(id: string, overrides: Partial<AttachmentRuleSuggestion> = {}): AttachmentRuleSuggestion {
  return {
    id,
    accountId: 'acc-1',
    name: 'Acme · invoice',
    senderEmailPattern: 'billing@acme.com',
    filenamePattern: 'Invoice_*.pdf',
    tags: ['invoice', 'acme'],
    emailCount: 4,
    firstSeen: 1_760_000_000,
    lastSeen: 1_770_000_000,
    sampleFilenames: ['Invoice_0042.pdf'],
    status: 'pending',
    createdAt: 1_770_000_000,
    updatedAt: 1_770_000_000,
    ...overrides,
  };
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.clearAllMocks();
  useAttachmentStore.setState({ suggestions: [], suggestionsAccountId: null, error: null });
});

describe('fetchSuggestions', () => {
  it('stores the pending suggestions of the account', async () => {
    vi.mocked(api.listAttachmentRuleSuggestions).mockResolvedValueOnce([makeSuggestion('s1')]);

    await useAttachmentStore.getState().fetchSuggestions('acc-1');

    expect(useAttachmentStore.getState().suggestions.map((s) => s.id)).toEqual(['s1']);
  });

  it('ignores a slower response for an account the user already left', async () => {
    const slow = deferred<AttachmentRuleSuggestion[]>();
    vi.mocked(api.listAttachmentRuleSuggestions)
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValueOnce([makeSuggestion('s2', { accountId: 'acc-2' })]);

    const first = useAttachmentStore.getState().fetchSuggestions('acc-1');
    await useAttachmentStore.getState().fetchSuggestions('acc-2');
    slow.resolve([makeSuggestion('s1')]);
    await first;

    expect(useAttachmentStore.getState().suggestions.map((s) => s.id)).toEqual(['s2']);
  });
});

describe('refreshSuggestions', () => {
  it('replaces the list with the re-mined suggestions', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('old')], suggestionsAccountId: 'acc-1' });
    vi.mocked(api.refreshAttachmentRuleSuggestions).mockResolvedValueOnce([makeSuggestion('new')]);

    await useAttachmentStore.getState().refreshSuggestions('acc-1');

    expect(useAttachmentStore.getState().suggestions.map((s) => s.id)).toEqual(['new']);
  });
});

describe('dismissSuggestion', () => {
  it('dismisses in the backend and drops only that suggestion', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1'), makeSuggestion('s2')] });

    await useAttachmentStore.getState().dismissSuggestion('acc-1', 's1');

    expect(api.dismissAttachmentRuleSuggestion).toHaveBeenCalledWith('acc-1', 's1');
    expect(useAttachmentStore.getState().suggestions.map((s) => s.id)).toEqual(['s2']);
  });

  it('keeps the suggestion and rejects when the backend fails', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')] });
    vi.mocked(api.dismissAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('db locked'));

    await expect(useAttachmentStore.getState().dismissSuggestion('acc-1', 's1')).rejects.toThrow('db locked');

    expect(useAttachmentStore.getState().suggestions).toHaveLength(1);
    expect(useAttachmentStore.getState().error).toContain('db locked');
  });

  it('is not brought back by a refresh that was already in flight', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')], suggestionsAccountId: 'acc-1' });
    const slow = deferred<AttachmentRuleSuggestion[]>();
    vi.mocked(api.refreshAttachmentRuleSuggestions).mockReturnValueOnce(slow.promise);

    const refresh = useAttachmentStore.getState().refreshSuggestions('acc-1');
    await useAttachmentStore.getState().dismissSuggestion('acc-1', 's1');
    slow.resolve([makeSuggestion('s1'), makeSuggestion('s2')]);
    await refresh;

    expect(useAttachmentStore.getState().suggestions.map((s) => s.id)).toEqual(['s2']);
  });
});

describe('switching account', () => {
  it("drops the previous account's suggestions before the new ones arrive", async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')], suggestionsAccountId: 'acc-1' });
    const slow = deferred<AttachmentRuleSuggestion[]>();
    vi.mocked(api.listAttachmentRuleSuggestions).mockReturnValueOnce(slow.promise);

    const load = useAttachmentStore.getState().fetchSuggestions('acc-2');

    expect(useAttachmentStore.getState().suggestions).toEqual([]);
    slow.resolve([]);
    await load;
  });

  it("does not keep the previous account's suggestions when the new load fails", async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')], suggestionsAccountId: 'acc-1' });
    vi.mocked(api.listAttachmentRuleSuggestions).mockRejectedValueOnce(new Error('offline'));

    await expect(useAttachmentStore.getState().fetchSuggestions('acc-2')).rejects.toThrow('offline');

    expect(useAttachmentStore.getState().suggestions).toEqual([]);
  });
});

describe('rule applies', () => {
  beforeEach(() => {
    useAttachmentStore.setState({ ruleApplies: {} });
  });

  it('gives each apply its own run id', () => {
    const { beginRuleApply } = useAttachmentStore.getState();

    expect(beginRuleApply('r1')).not.toBe(beginRuleApply('r1'));
  });

  it('tracks progress of the running apply', () => {
    const { beginRuleApply, reportRuleApplyProgress } = useAttachmentStore.getState();
    const runId = beginRuleApply('r1');

    reportRuleApplyProgress({ ruleId: 'r1', runId, processed: 3, total: 10, saved: 1 });

    expect(useAttachmentStore.getState().ruleApplies.r1).toMatchObject({
      processed: 3,
      total: 10,
      saved: 1,
      status: 'running',
    });
  });

  it('ignores progress of a superseded run', () => {
    const { beginRuleApply, reportRuleApplyProgress } = useAttachmentStore.getState();
    const older = beginRuleApply('r1');
    beginRuleApply('r1');

    reportRuleApplyProgress({ ruleId: 'r1', runId: older, processed: 9, total: 10, saved: 9 });

    expect(useAttachmentStore.getState().ruleApplies.r1.processed).toBe(0);
  });

  it('ignores progress for a rule that is not running', () => {
    useAttachmentStore
      .getState()
      .reportRuleApplyProgress({ ruleId: 'r1', runId: 'x', processed: 3, total: 10, saved: 1 });

    expect(useAttachmentStore.getState().ruleApplies.r1).toBeUndefined();
  });

  it('a superseded run finishing does not overwrite the newer run', () => {
    const { beginRuleApply, finishRuleApply } = useAttachmentStore.getState();
    const older = beginRuleApply('r1');
    beginRuleApply('r1');

    finishRuleApply('r1', older, 5, 5);

    expect(useAttachmentStore.getState().ruleApplies.r1.status).toBe('running');
  });

  it('a cancelled run leaves no state behind', () => {
    const { beginRuleApply, dropRuleApply } = useAttachmentStore.getState();
    const run = beginRuleApply('r1');

    dropRuleApply('r1', run);

    expect(useAttachmentStore.getState().ruleApplies.r1).toBeUndefined();
  });

  it('records the outcome of the current run', () => {
    const { beginRuleApply, finishRuleApply, failRuleApply } = useAttachmentStore.getState();
    const run = beginRuleApply('r1');
    finishRuleApply('r1', run, 2, 7);
    expect(useAttachmentStore.getState().ruleApplies.r1).toMatchObject({ status: 'done', saved: 2, collected: 7 });

    const again = beginRuleApply('r1');
    failRuleApply('r1', again);
    expect(useAttachmentStore.getState().ruleApplies.r1.status).toBe('failed');
  });
});

describe('acceptSuggestion', () => {
  it('rejects when the backend fails', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')] });
    vi.mocked(api.acceptAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('gone'));

    await expect(useAttachmentStore.getState().acceptSuggestion('acc-1', 's1')).rejects.toThrow('gone');
  });

  it('marks it accepted and removes it from the pending list', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')] });

    await useAttachmentStore.getState().acceptSuggestion('acc-1', 's1');

    expect(api.acceptAttachmentRuleSuggestion).toHaveBeenCalledWith('acc-1', 's1');
    expect(useAttachmentStore.getState().suggestions).toEqual([]);
  });
});

describe('selectSuggestionCount', () => {
  it('counts pending suggestions for the badge', () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1'), makeSuggestion('s2')] });
    expect(selectSuggestionCount(useAttachmentStore.getState())).toBe(2);
  });
});
