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
import { useAccountStore } from '@/stores/accountStore';
import { useAttachmentStore } from '@/stores/attachmentStore';
import type { Account } from '@/types';
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
  useAttachmentStore.setState({ suggestions: [], suggestionsAccountId: null, dismissedSuggestions: [], error: null });
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
});
