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

  it('keeps the suggestion and surfaces the error when the backend fails', async () => {
    useAttachmentStore.setState({ suggestions: [makeSuggestion('s1')] });
    vi.mocked(api.dismissAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('db locked'));

    await useAttachmentStore.getState().dismissSuggestion('acc-1', 's1');

    expect(useAttachmentStore.getState().suggestions).toHaveLength(1);
    expect(useAttachmentStore.getState().error).toContain('db locked');
  });
});

describe('acceptSuggestion', () => {
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
