// Unit tests for the attachment-rule suggestion slice of attachmentStore:
// async actions with the api layer mocked, plus the count selector.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { AttachmentRuleSuggestion } from '@/types';
import { selectSuggestionCount, useAttachmentStore } from './attachmentStore';

vi.mock('@/lib/api', () => ({
  getAttachmentTags: vi.fn(async () => []),
  listAttachmentRules: vi.fn(async () => []),
  createAttachmentRule: vi.fn(),
  updateAttachmentRule: vi.fn(),
  deleteAttachmentRule: vi.fn(async () => {}),
  listDismissedAttachmentRuleSuggestions: vi.fn(async () => []),
  restoreAttachmentRuleSuggestion: vi.fn(async () => []),
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
  useAttachmentStore.setState({
    suggestions: [],
    suggestionsAccountId: null,
    dismissedSuggestions: [],
    suggestionsLoading: false,
    error: null,
  });
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

describe('dismissed suggestions', () => {
  it('a dismissal moves the suggestion to the dismissed list', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('d1')], suggestionsAccountId: 'acc-1' });

    await useAttachmentStore.getState().dismissSuggestion('acc-1', 'd1');

    expect(useAttachmentStore.getState().dismissedSuggestions.map((s) => s.id)).toEqual(['d1']);
  });

  it('loads the dismissed suggestions of the account', async () => {
    vi.mocked(api.listDismissedAttachmentRuleSuggestions).mockResolvedValueOnce([makeSuggestion('d2')]);

    await useAttachmentStore.getState().fetchDismissedSuggestions('acc-1');

    expect(useAttachmentStore.getState().dismissedSuggestions.map((s) => s.id)).toEqual(['d2']);
  });

  it('restoring puts the suggestion back in the pending list', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('d3')], suggestionsAccountId: 'acc-1' });
    await useAttachmentStore.getState().dismissSuggestion('acc-1', 'd3');
    vi.mocked(api.restoreAttachmentRuleSuggestion).mockResolvedValueOnce([makeSuggestion('d3')]);

    await useAttachmentStore.getState().restoreSuggestion('acc-1', 'd3');

    expect(api.restoreAttachmentRuleSuggestion).toHaveBeenCalledWith('acc-1', 'd3');
    expect(useAttachmentStore.getState().suggestions.map((s) => s.id)).toEqual(['d3']);
    expect(useAttachmentStore.getState().dismissedSuggestions).toEqual([]);
  });
});

describe('suggestionsLoading', () => {
  it('is set while a re-mine runs', async () => {
    const slow = deferred<AttachmentRuleSuggestion[]>();
    vi.mocked(api.refreshAttachmentRuleSuggestions).mockReturnValueOnce(slow.promise);

    const refresh = useAttachmentStore.getState().refreshSuggestions('acc-1');
    expect(useAttachmentStore.getState().suggestionsLoading).toBe(true);
    slow.resolve([]);
    await refresh;

    expect(useAttachmentStore.getState().suggestionsLoading).toBe(false);
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

describe('rules', () => {
  const rule = (id: string, name = 'Acme') => ({
    id,
    accountId: 'acc-1',
    name,
    senderEmailPattern: 'billing@acme.com',
    subjectPattern: null,
    filenamePattern: null,
    tags: [],
    enabled: true,
    createdAt: 0,
    updatedAt: 0,
  });

  beforeEach(() => {
    useAttachmentStore.setState({ rules: [], isLoadingRules: false });
  });

  it('loads the account rules', async () => {
    vi.mocked(api.listAttachmentRules).mockResolvedValueOnce([rule('r1')]);

    await useAttachmentStore.getState().fetchRules('acc-1');

    expect(useAttachmentStore.getState().rules.map((r) => r.id)).toEqual(['r1']);
    expect(useAttachmentStore.getState().isLoadingRules).toBe(false);
  });

  it('a failed rules load surfaces the error and stops loading', async () => {
    vi.mocked(api.listAttachmentRules).mockRejectedValueOnce(new Error('db locked'));

    await useAttachmentStore.getState().fetchRules('acc-1');

    expect(useAttachmentStore.getState().error).toContain('db locked');
    expect(useAttachmentStore.getState().isLoadingRules).toBe(false);
  });

  it('a created rule goes first', async () => {
    useAttachmentStore.setState({ rules: [rule('old')] });
    vi.mocked(api.createAttachmentRule).mockResolvedValueOnce(rule('new'));

    await useAttachmentStore.getState().createRule('acc-1', 'Acme', 'billing@acme.com', null, null, []);

    expect(useAttachmentStore.getState().rules.map((r) => r.id)).toEqual(['new', 'old']);
  });

  it('an updated rule replaces its old version', async () => {
    useAttachmentStore.setState({ rules: [rule('r1', 'Old name')] });
    vi.mocked(api.updateAttachmentRule).mockResolvedValueOnce(rule('r1', 'New name'));

    await useAttachmentStore
      .getState()
      .updateRule('acct-1', 'r1', 'New name', 'billing@acme.com', null, null, [], true);

    expect(useAttachmentStore.getState().rules.map((r) => r.name)).toEqual(['New name']);
  });

  it('a deleted rule leaves the list', async () => {
    useAttachmentStore.setState({ rules: [rule('r1'), rule('r2')] });

    await useAttachmentStore.getState().deleteRule('r1', 'acc-1');

    expect(useAttachmentStore.getState().rules.map((r) => r.id)).toEqual(['r2']);
  });
});

describe('suggestion failures', () => {
  it('a failed load of dismissed suggestions rejects and surfaces the error', async () => {
    vi.mocked(api.listDismissedAttachmentRuleSuggestions).mockRejectedValueOnce(new Error('offline'));

    await expect(useAttachmentStore.getState().fetchDismissedSuggestions('acc-1')).rejects.toThrow('offline');
    expect(useAttachmentStore.getState().error).toContain('offline');
  });

  it('a failed restore rejects and keeps the suggestion dismissed', async () => {
    useAttachmentStore.setState({ dismissedSuggestions: [makeSuggestion('d1', { status: 'dismissed' })] });
    vi.mocked(api.restoreAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('gone'));

    await expect(useAttachmentStore.getState().restoreSuggestion('acc-1', 'd1')).rejects.toThrow('gone');
    expect(useAttachmentStore.getState().dismissedSuggestions.map((s) => s.id)).toEqual(['d1']);
  });
});

describe('tag filter', () => {
  beforeEach(() => {
    useAttachmentStore.setState({ selectedTag: null, availableTags: [] });
  });

  it("drops a selected tag the account's attachments no longer carry", async () => {
    // Picked on another account, or its last attachment went with a deleted rule.
    useAttachmentStore.setState({ selectedTag: 'facturas' });
    vi.mocked(api.getAttachmentTags).mockResolvedValueOnce(['acme', 'invoice']);

    await useAttachmentStore.getState().fetchTags('acc-1');

    expect(useAttachmentStore.getState().selectedTag).toBeNull();
  });

  it('drops it too when the account has no tags at all', async () => {
    useAttachmentStore.setState({ selectedTag: 'facturas' });
    vi.mocked(api.getAttachmentTags).mockResolvedValueOnce([]);

    await useAttachmentStore.getState().fetchTags('acc-1');

    expect(useAttachmentStore.getState().selectedTag).toBeNull();
  });

  it('keeps a selected tag the account still has', async () => {
    useAttachmentStore.setState({ selectedTag: 'invoice' });
    vi.mocked(api.getAttachmentTags).mockResolvedValueOnce(['acme', 'invoice']);

    await useAttachmentStore.getState().fetchTags('acc-1');

    expect(useAttachmentStore.getState().selectedTag).toBe('invoice');
  });
});
