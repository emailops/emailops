// Unit tests for the attachment-rule suggestion slice of attachmentStore:
// async actions with the api layer mocked, plus the count selector.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Attachment, AttachmentRuleSuggestion } from '@/types';
import { isAllChecked, selectSuggestionCount, useAttachmentStore } from './attachmentStore';

vi.mock('@/lib/api', () => ({
  getAttachmentTags: vi.fn(async () => []),
  getAttachments: vi.fn(async () => []),
  countAttachments: vi.fn(async () => 0),
  listAttachmentIds: vi.fn(async () => []),
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
    useAttachmentStore.setState({ rules: [], rulesAccountId: null, isLoadingRules: false });
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

  it('drops a slower rules response for an account the user already left', async () => {
    const slow = deferred<ReturnType<typeof rule>[]>();
    vi.mocked(api.listAttachmentRules)
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValueOnce([{ ...rule('rb'), accountId: 'acc-2' }]);

    const first = useAttachmentStore.getState().fetchRules('acc-1');
    await useAttachmentStore.getState().fetchRules('acc-2');
    slow.resolve([rule('ra')]);
    await first;

    const state = useAttachmentStore.getState();
    expect(state.rules.map((r) => r.id)).toEqual(['rb']);
    expect(state.rulesAccountId).toBe('acc-2');
    expect(state.isLoadingRules).toBe(false);
  });

  it("clears the previous account's rules while the new ones load", async () => {
    useAttachmentStore.setState({ rules: [rule('ra')], rulesAccountId: 'acc-1' });
    vi.mocked(api.listAttachmentRules).mockReturnValueOnce(new Promise(() => {}));

    void useAttachmentStore.getState().fetchRules('acc-2');

    expect(useAttachmentStore.getState().rules).toEqual([]);
    expect(useAttachmentStore.getState().isLoadingRules).toBe(true);
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

  it('keeps the selected tag when the new scope spells it differently, under the new spelling', async () => {
    useAttachmentStore.setState({ selectedTag: 'Invoice' });
    vi.mocked(api.getAttachmentTags).mockResolvedValueOnce(['acme', 'invoice']);

    await useAttachmentStore.getState().fetchTags(null);

    expect(useAttachmentStore.getState().selectedTag).toBe('invoice');
  });

  it('a failed tag load rejects and keeps the current tags', async () => {
    useAttachmentStore.setState({ selectedTag: 'invoice', availableTags: ['invoice'] });
    vi.mocked(api.getAttachmentTags).mockRejectedValueOnce(new Error('db locked'));

    await expect(useAttachmentStore.getState().fetchTags('acc-1')).rejects.toThrow('db locked');
    expect(useAttachmentStore.getState().availableTags).toEqual(['invoice']);
    expect(useAttachmentStore.getState().selectedTag).toBe('invoice');
  });
});

describe('isAllChecked', () => {
  it.each([
    { checked: 0, total: 0, want: false },
    { checked: 0, total: 3, want: false },
    { checked: 2, total: 3, want: false },
    { checked: 3, total: 3, want: true },
  ])('$checked checked of $total → $want', ({ checked, total, want }) => {
    const ids = new Set(Array.from({ length: checked }, (_, i) => `a${i}`));
    expect(isAllChecked(ids, total)).toBe(want);
  });
});

describe('attachment list', () => {
  const ACCOUNT_KEY = 'account:acc-1';

  const attachment = (id: string): Attachment => ({
    id,
    accountId: 'acc-1',
    emailId: `email-${id}`,
    ruleId: 'r1',
    ruleName: 'Invoices',
    gmailAttachmentId: '',
    filename: `${id}.pdf`,
    mimeType: 'application/pdf',
    fileSize: 1,
    filePath: `attachments/acc-1/${id}`,
    tags: ['invoice'],
    senderEmail: 'billing@vendor.example',
    subject: 'Invoice',
    emailTimestamp: 0,
    createdAt: 0,
  });
  const page = (from: number, n: number) => Array.from({ length: n }, (_, i) => attachment(`a${from + i}`));

  beforeEach(() => {
    useAttachmentStore.getState().reset();
    vi.mocked(api.getAttachments).mockReset();
    vi.mocked(api.countAttachments).mockReset();
    vi.mocked(api.listAttachmentIds).mockReset();
  });

  async function load(rows: Attachment[], total: number, tag: string | null = 'invoice', scopeKey = ACCOUNT_KEY) {
    vi.mocked(api.getAttachments).mockResolvedValueOnce(rows);
    vi.mocked(api.countAttachments).mockResolvedValueOnce(total);
    await useAttachmentStore.getState().fetchAttachments('acc-1', tag, scopeKey);
  }

  describe('select all', () => {
    it('marks every attachment of the filter, not only the loaded page', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.listAttachmentIds).mockResolvedValueOnce(page(0, 120).map((a) => a.id));

      await useAttachmentStore.getState().toggleSelectAll();

      const { checkedIds, totalCount } = useAttachmentStore.getState();
      expect(api.listAttachmentIds).toHaveBeenCalledWith('acc-1', 'invoice');
      expect(checkedIds.size).toBe(120);
      expect(isAllChecked(checkedIds, totalCount)).toBe(true);
    });

    it('a second click clears the selection without asking the backend', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.listAttachmentIds).mockResolvedValueOnce(page(0, 120).map((a) => a.id));
      await useAttachmentStore.getState().toggleSelectAll();

      await useAttachmentStore.getState().toggleSelectAll();

      expect(useAttachmentStore.getState().checkedIds.size).toBe(0);
      expect(api.listAttachmentIds).toHaveBeenCalledTimes(1);
    });

    it('unchecking one row leaves the box unchecked', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.listAttachmentIds).mockResolvedValueOnce(page(0, 120).map((a) => a.id));
      await useAttachmentStore.getState().toggleSelectAll();

      useAttachmentStore.getState().toggleChecked('a7');

      const { checkedIds, totalCount } = useAttachmentStore.getState();
      expect(checkedIds.size).toBe(119);
      expect(isAllChecked(checkedIds, totalCount)).toBe(false);
    });

    it('drops the ids when the filter changed while they loaded', async () => {
      await load(page(0, 50), 120);
      const ids = deferred<string[]>();
      vi.mocked(api.listAttachmentIds).mockReturnValueOnce(ids.promise);

      const selecting = useAttachmentStore.getState().toggleSelectAll();
      await load(page(0, 3), 3, 'receipt');
      ids.resolve(page(0, 120).map((a) => a.id));
      await selecting;

      expect(useAttachmentStore.getState().checkedIds.size).toBe(0);
      expect(useAttachmentStore.getState().isSelectingAll).toBe(false);
    });

    it('keeps the ids when a background refresh of the same list lands meanwhile', async () => {
      await load(page(0, 50), 120);
      const ids = deferred<string[]>();
      vi.mocked(api.listAttachmentIds).mockReturnValueOnce(ids.promise);

      const selecting = useAttachmentStore.getState().toggleSelectAll();
      await load(page(0, 50), 121);
      ids.resolve(page(0, 121).map((a) => a.id));
      await selecting;

      expect(useAttachmentStore.getState().checkedIds.size).toBe(121);
    });

    it('a failure stops waiting and shows the list error', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.listAttachmentIds).mockRejectedValueOnce(new Error('db locked'));

      await useAttachmentStore.getState().toggleSelectAll();

      expect(useAttachmentStore.getState().isSelectingAll).toBe(false);
      expect(useAttachmentStore.getState().listError).toContain('db locked');
    });
  });

  describe('refresh', () => {
    it('a refresh of the same list keeps the selection, the viewer and every loaded row', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.getAttachments).mockResolvedValueOnce(page(50, 30));
      await useAttachmentStore.getState().loadMoreAttachments();
      useAttachmentStore.getState().toggleChecked('a3');
      useAttachmentStore.getState().selectAttachment(attachment('a3'));

      const fresh = deferred<Attachment[]>();
      vi.mocked(api.getAttachments).mockReturnValueOnce(fresh.promise);
      vi.mocked(api.countAttachments).mockResolvedValueOnce(81);
      const refreshing = useAttachmentStore.getState().fetchAttachments('acc-1', 'invoice', ACCOUNT_KEY);

      expect(useAttachmentStore.getState().attachments).toHaveLength(80);
      fresh.resolve([attachment('new'), ...page(0, 80)]);
      await refreshing;

      const state = useAttachmentStore.getState();
      expect(api.getAttachments).toHaveBeenLastCalledWith('acc-1', 'invoice', 80, 0);
      expect(state.attachments).toHaveLength(81);
      expect(state.checkedIds.has('a3')).toBe(true);
      expect(state.selectedAttachment?.id).toBe('a3');
    });

    it('another tag starts a new list: rows, selection and viewer are cleared', async () => {
      await load(page(0, 50), 120);
      useAttachmentStore.getState().toggleChecked('a3');
      useAttachmentStore.getState().selectAttachment(attachment('a3'));
      vi.mocked(api.getAttachments).mockReturnValueOnce(new Promise(() => {}));
      vi.mocked(api.countAttachments).mockReturnValueOnce(new Promise(() => {}));

      void useAttachmentStore.getState().fetchAttachments('acc-1', 'receipt', ACCOUNT_KEY);

      const state = useAttachmentStore.getState();
      expect(state.attachments).toEqual([]);
      expect(state.checkedIds.size).toBe(0);
      expect(state.selectedAttachment).toBeNull();
    });

    it('another scope starts a new list too', async () => {
      await load(page(0, 50), 120);
      useAttachmentStore.getState().toggleChecked('a3');

      await load(page(0, 2), 2, 'invoice', 'all:acc-1,acc-2');

      expect(useAttachmentStore.getState().checkedIds.size).toBe(0);
    });

    it('a failed load shows the list error, and the next good load clears it', async () => {
      vi.mocked(api.getAttachments).mockRejectedValueOnce(new Error('db locked'));
      vi.mocked(api.countAttachments).mockResolvedValueOnce(0);
      await useAttachmentStore.getState().fetchAttachments('acc-1', null, ACCOUNT_KEY);
      expect(useAttachmentStore.getState().listError).toContain('db locked');

      await load(page(0, 2), 2, null);

      expect(useAttachmentStore.getState().listError).toBeNull();
    });
  });

  describe('load more', () => {
    it('appends the next page', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.getAttachments).mockResolvedValueOnce(page(50, 50));

      await useAttachmentStore.getState().loadMoreAttachments();

      expect(api.getAttachments).toHaveBeenLastCalledWith('acc-1', 'invoice', 50, 50);
      expect(useAttachmentStore.getState().attachments).toHaveLength(100);
      expect(useAttachmentStore.getState().hasMore).toBe(true);
    });

    it('drops a page of a list the user already left', async () => {
      await load(page(0, 50), 120);
      const more = deferred<Attachment[]>();
      vi.mocked(api.getAttachments).mockReturnValueOnce(more.promise);

      const loading = useAttachmentStore.getState().loadMoreAttachments();
      await load(page(0, 3), 3, 'receipt');
      more.resolve(page(50, 50));
      await loading;

      expect(useAttachmentStore.getState().attachments).toHaveLength(3);
      expect(useAttachmentStore.getState().isLoadingMore).toBe(false);
    });

    it('does not repeat a row that shifted into the next page', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.getAttachments).mockResolvedValueOnce([attachment('a49'), ...page(50, 49)]);

      await useAttachmentStore.getState().loadMoreAttachments();

      const ids = useAttachmentStore.getState().attachments.map((a) => a.id);
      expect(ids).toHaveLength(99);
      expect(new Set(ids).size).toBe(99);
    });

    it('a short page ends the list even if the count promised more', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.getAttachments).mockResolvedValueOnce(page(50, 10));

      await useAttachmentStore.getState().loadMoreAttachments();

      expect(useAttachmentStore.getState().hasMore).toBe(false);
    });

    it('a failure keeps the rows, leaves the list error alone and rejects', async () => {
      await load(page(0, 50), 120);
      vi.mocked(api.getAttachments).mockRejectedValueOnce(new Error('offline'));

      await expect(useAttachmentStore.getState().loadMoreAttachments()).rejects.toThrow('offline');

      const state = useAttachmentStore.getState();
      expect(state.attachments).toHaveLength(50);
      expect(state.listError).toBeNull();
      expect(state.isLoadingMore).toBe(false);
    });
  });
});
