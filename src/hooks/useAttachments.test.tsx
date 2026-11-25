// useAttachments wires the attachment store to the active account and to the
// backend's events, and is where suggestion actions are logged.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const events = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void> }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => void) => {
    events.handlers[name] = handler;
    return () => {};
  }),
}));

const logs = vi.hoisted(() => ({ addLog: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof logs.addLog }) => unknown) => selector({ addLog: logs.addLog }),
}));

vi.mock('@/lib/api', () => ({
  listAttachmentRules: vi.fn(async () => []),
  getAttachmentTags: vi.fn(async () => []),
  getAttachments: vi.fn(async () => []),
  countAttachments: vi.fn(async () => 0),
  listAttachmentIds: vi.fn(async () => []),
  countAttachmentsForRule: vi.fn(async () => 0),
  listAttachmentRuleSuggestions: vi.fn(async () => []),
  refreshAttachmentRuleSuggestions: vi.fn(async () => []),
  listDismissedAttachmentRuleSuggestions: vi.fn(async () => []),
  dismissAttachmentRuleSuggestion: vi.fn(async () => {}),
  acceptAttachmentRuleSuggestion: vi.fn(async () => {}),
  restoreAttachmentRuleSuggestion: vi.fn(async () => []),
  createAttachmentRule: vi.fn(async () => ({
    id: 'r1',
    accountId: 'acc-1',
    name: 'Acme',
    senderEmailPattern: 'billing@acme.com',
    subjectPattern: null,
    filenamePattern: null,
    tags: [],
    enabled: true,
    createdAt: 0,
    updatedAt: 0,
  })),
}));

import * as api from '@/lib/api';
import { ALL_ACCOUNTS_ID, useAccountStore } from '@/stores/accountStore';
import { useAttachmentStore } from '@/stores/attachmentStore';
import type { Account, AttachmentRule } from '@/types';
import { useAttachments } from './useAttachments';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let hook: ReturnType<typeof useAttachments>;
function Probe() {
  hook = useAttachments();
  return null;
}

let container: HTMLDivElement;
let root: Root;

async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

beforeEach(async () => {
  vi.clearAllMocks();
  useAttachmentStore.getState().reset();
  useAttachmentStore.setState({
    suggestions: [],
    suggestionsAccountId: null,
    dismissedSuggestions: [],
    error: null,
    selectedTag: null,
  });
  useAccountStore.setState({
    accounts: [{ id: 'acc-1', email: 'me@example.com', enabled: true } as unknown as Account],
    activeAccountId: 'acc-1',
  });
  container = document.createElement('div');
  root = createRoot(container);
  await act(async () => root.render(<Probe />));
  await flush();
});

afterEach(() => {
  act(() => root.unmount());
});

describe('useAttachments', () => {
  it('loads the rules, tags and suggestions of the active account', () => {
    expect(api.listAttachmentRules).toHaveBeenCalledWith('acc-1');
    expect(api.getAttachmentTags).toHaveBeenCalledWith('acc-1');
    expect(api.listAttachmentRuleSuggestions).toHaveBeenCalledWith('acc-1');
  });

  it('reloads suggestions when the backend re-mined this account', async () => {
    vi.mocked(api.listAttachmentRuleSuggestions).mockClear();

    await act(async () => events.handlers['attachment-rule-suggestions-updated']?.({ payload: 'acc-1' }));

    expect(api.listAttachmentRuleSuggestions).toHaveBeenCalledWith('acc-1');
  });

  it('ignores a re-mine of another account', async () => {
    vi.mocked(api.listAttachmentRuleSuggestions).mockClear();

    await act(async () => events.handlers['attachment-rule-suggestions-updated']?.({ payload: 'acc-2' }));

    expect(api.listAttachmentRuleSuggestions).not.toHaveBeenCalled();
  });

  it('a failed background suggestion load goes to the output panel', async () => {
    vi.mocked(api.listAttachmentRuleSuggestions).mockRejectedValueOnce(new Error('offline'));

    await act(async () => events.handlers['attachment-rule-suggestions-updated']?.({ payload: 'acc-1' }));
    await flush();

    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('offline'));
  });

  it('opening the rules re-mines and loads the dismissed suggestions', async () => {
    await act(async () => hook.refreshSuggestions());

    expect(api.refreshAttachmentRuleSuggestions).toHaveBeenCalledWith('acc-1');
    expect(api.listDismissedAttachmentRuleSuggestions).toHaveBeenCalledWith('acc-1');
  });

  it('a dismissal is logged', async () => {
    await act(async () => hook.dismissSuggestion('s1'));

    expect(logs.addLog).toHaveBeenCalledWith('success', 'attachments', expect.any(String));
  });

  it('a failed dismissal is logged and rethrown', async () => {
    vi.mocked(api.dismissAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('db locked'));

    await act(async () => {
      await expect(hook.dismissSuggestion('s1')).rejects.toThrow('db locked');
    });

    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('db locked'));
    expect(logs.addLog).not.toHaveBeenCalledWith('success', expect.anything(), expect.anything());
  });

  it('a failed accept is logged and rethrown', async () => {
    vi.mocked(api.acceptAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('gone'));

    await act(async () => {
      await expect(hook.acceptSuggestion('s1')).rejects.toThrow('gone');
    });

    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('gone'));
  });

  it('a restore is logged, and a failed one rethrown', async () => {
    await act(async () => hook.restoreSuggestion('s1'));
    expect(logs.addLog).toHaveBeenCalledWith('success', 'attachments', expect.any(String));

    vi.mocked(api.restoreAttachmentRuleSuggestion).mockRejectedValueOnce(new Error('gone'));
    await act(async () => {
      await expect(hook.restoreSuggestion('s1')).rejects.toThrow('gone');
    });
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('gone'));
  });

  it('creating a rule re-mines, so a suggestion it covers leaves the list', async () => {
    await act(async () => {
      await hook.createRule('Acme', 'billing@acme.com', null, null, []);
    });

    expect(api.refreshAttachmentRuleSuggestions).toHaveBeenCalledWith('acc-1');
  });

  it('a tag filter left over from another account falls back to all attachments', async () => {
    act(() => root.unmount());
    useAttachmentStore.setState({ selectedTag: 'facturas' });
    vi.mocked(api.getAttachmentTags).mockResolvedValueOnce([]);
    vi.mocked(api.getAttachments).mockClear();
    root = createRoot(container);

    await act(async () => root.render(<Probe />));
    await flush();

    expect(useAttachmentStore.getState().selectedTag).toBeNull();
    const calls = vi.mocked(api.getAttachments).mock.calls;
    expect(calls[calls.length - 1]?.slice(0, 2)).toEqual(['acc-1', null]);
  });

  it('a failed tag load goes to the output panel and the list still loads', async () => {
    vi.mocked(api.getAttachmentTags).mockRejectedValueOnce(new Error('db locked'));
    vi.mocked(api.getAttachments).mockClear();

    await act(async () => hook.retry());
    await flush();

    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('db locked'));
    expect(api.getAttachments).toHaveBeenCalled();
  });

  it('a failed list load goes to the output panel', async () => {
    vi.mocked(api.getAttachments).mockRejectedValueOnce(new Error('disk I/O error'));

    await act(async () => hook.retry());
    await flush();

    expect(useAttachmentStore.getState().listError).toContain('disk I/O error');
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('disk I/O error'));
  });

  it('a failed load of more rows goes to the output panel', async () => {
    useAttachmentStore.setState({ hasMore: true, totalCount: 120 });
    vi.mocked(api.getAttachments).mockRejectedValueOnce(new Error('offline'));

    await act(async () => hook.loadMore());
    await flush();

    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('offline'));
  });

  it('retry reloads the list and the tags', async () => {
    vi.mocked(api.getAttachments).mockClear();
    vi.mocked(api.getAttachmentTags).mockClear();

    await act(async () => hook.retry());

    expect(api.getAttachments).toHaveBeenCalledWith('acc-1', null, 50, 0);
    expect(api.getAttachmentTags).toHaveBeenCalledWith('acc-1');
  });
});

describe('useAttachments in All accounts', () => {
  const account = (id: string, email: string, enabled = true) => ({ id, email, enabled }) as unknown as Account;
  const twoAccounts = () => [
    account('acc-1', 'a@example.com'),
    account('acc-2', 'b@example.com'),
    account('acc-3', 'c@example.com', false),
  ];

  async function mountAll(accounts: Account[] = twoAccounts()) {
    act(() => root.unmount());
    vi.clearAllMocks();
    useAttachmentStore.getState().reset();
    useAccountStore.setState({ accounts, activeAccountId: ALL_ACCOUNTS_ID });
    root = createRoot(container);
    await act(async () => root.render(<Probe />));
    await flush();
  }

  afterEach(() => {
    vi.useRealTimers();
  });

  it('asks for the attachments and tags of every enabled account', async () => {
    await mountAll();

    expect(api.getAttachments).toHaveBeenCalledWith(null, null, 50, 0);
    expect(api.countAttachments).toHaveBeenCalledWith(null, null);
    expect(api.getAttachmentTags).toHaveBeenCalledWith(null);
  });

  it('asks for nothing before the accounts load', async () => {
    await mountAll([]);

    expect(api.getAttachments).not.toHaveBeenCalled();
    expect(api.getAttachmentTags).not.toHaveBeenCalled();
  });

  it('groups attachment events of enabled accounts into one reload', async () => {
    await mountAll();
    vi.mocked(api.getAttachments).mockClear();
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });

    act(() => events.handlers['attachments-updated']?.({ payload: 'acc-1' }));
    act(() => vi.advanceTimersByTime(100));
    act(() => events.handlers['attachments-updated']?.({ payload: 'acc-2' }));
    act(() => vi.advanceTimersByTime(200));
    act(() => events.handlers['attachments-updated']?.({ payload: 'acc-1' }));
    act(() => vi.advanceTimersByTime(499));
    expect(api.getAttachments).not.toHaveBeenCalled();

    act(() => vi.advanceTimersByTime(1));
    expect(api.getAttachments).toHaveBeenCalledTimes(1);
    expect(api.getAttachments).toHaveBeenCalledWith(null, null, 50, 0);
  });

  it('ignores attachment events of a disabled account', async () => {
    await mountAll();
    vi.mocked(api.getAttachments).mockClear();
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });

    act(() => events.handlers['attachments-updated']?.({ payload: 'acc-3' }));
    act(() => vi.advanceTimersByTime(1000));

    expect(api.getAttachments).not.toHaveBeenCalled();
  });

  it('enabling an account starts a new list and clears the selection', async () => {
    await mountAll();
    act(() => useAttachmentStore.getState().toggleChecked('att-1'));
    vi.mocked(api.getAttachments).mockClear();

    await act(async () =>
      useAccountStore.setState({ accounts: twoAccounts().map((a) => ({ ...a, enabled: true }) as Account) }),
    );
    await flush();

    expect(api.getAttachments).toHaveBeenCalledWith(null, null, 50, 0);
    expect(useAttachmentStore.getState().checkedIds.size).toBe(0);
  });

  it('rules follow the effective account, or the account of the email the rules were opened from', async () => {
    await mountAll();
    expect(api.listAttachmentRules).toHaveBeenCalledWith('acc-1');
    expect(hook.rulesAccountId).toBe('acc-1');

    await act(async () => hook.setRuleAccountFromEmail('acc-2'));
    await flush();
    expect(api.listAttachmentRules).toHaveBeenLastCalledWith('acc-2');
    expect(api.listAttachmentRuleSuggestions).toHaveBeenLastCalledWith('acc-2');

    await act(async () => {
      await hook.createRule('Sent invoices', 'b@example.com', null, '*invoice*', ['invoice']);
    });
    expect(vi.mocked(api.createAttachmentRule).mock.calls[0][0]).toBe('acc-2');

    await act(async () => hook.setRuleAccountFromEmail(null));
    await flush();
    expect(api.listAttachmentRules).toHaveBeenLastCalledWith('acc-1');
  });

  it("shows no rules of another account while the email's account loads", async () => {
    await mountAll();
    vi.mocked(api.listAttachmentRules).mockReturnValueOnce(new Promise(() => {}));
    useAttachmentStore.setState({
      rules: [{ id: 'ra', accountId: 'acc-1' } as unknown as AttachmentRule],
      rulesAccountId: 'acc-1',
    });

    await act(async () => hook.setRuleAccountFromEmail('acc-2'));

    expect(hook.rules).toEqual([]);
    expect(hook.isLoadingRules).toBe(true);
  });
});
