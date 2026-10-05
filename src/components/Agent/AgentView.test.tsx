import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentAction, AgentOverview, AgentRun } from '@/types';

// Stable across renders, like the real `t`.
const translation = {
  t: (key: string, opts?: Record<string, unknown>) => (opts?.count !== undefined ? `${key}:${opts.count}` : key),
  i18n: { language: 'en' },
};
vi.mock('react-i18next', () => ({ useTranslation: () => translation }));

const addLog = vi.fn();
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLog }) => unknown) => selector({ addLog }),
}));
vi.mock('@/stores/accountStore', () => ({
  useAccountStore: (selector: (s: { accounts: unknown[] }) => unknown) => selector({ accounts: [] }),
}));
vi.mock('@/components/Chat/EmailRefPill', () => ({
  EmailRefPill: ({ label }: { label: string }) => <span>{label}</span>,
}));
vi.mock('@/components/Chat/DraftRefPill', () => ({
  DraftRefPill: ({ draftId }: { draftId: string }) => <span data-testid={`draft-${draftId}`} />,
}));
vi.mock('@/components/shared/EmailPreviewById', () => ({
  EmailPreviewById: ({ emailId }: { emailId: string }) => <div data-testid={`preview-${emailId}`} />,
}));
const outboxSend =
  vi.fn<(message: { body: string; replyToEmailId?: string | null }, opts: { draftId?: string }) => Promise<string>>();
vi.mock('@/stores/outboxStore', () => ({
  useOutboxStore: { getState: () => ({ send: outboxSend }) },
}));

let agentUpdated: (() => void) | null = null;
vi.mock('@tauri-apps/api/event', () => ({
  listen: (name: string, handler: () => void) => {
    if (name === 'agent-updated') agentUpdated = handler;
    return Promise.resolve(() => {
      agentUpdated = null;
    });
  },
}));

const getAgentOverview = vi.fn<() => Promise<AgentOverview>>();
const setAgentEnabled = vi.fn<(enabled: boolean) => Promise<void>>();
const approveAgentAction = vi.fn<(id: string) => Promise<void>>();
const rejectAgentAction = vi.fn<(id: string) => Promise<void>>();
const getDraft = vi.fn();
const saveDraft = vi.fn();
const deleteDraft = vi.fn();
const reviewAgentDraft = vi.fn<(id: string, outcome: string) => Promise<void>>();
vi.mock('@/lib/api', () => ({
  getAgentOverview: () => getAgentOverview(),
  setAgentEnabled: (e: boolean) => setAgentEnabled(e),
  approveAgentAction: (id: string) => approveAgentAction(id),
  rejectAgentAction: (id: string) => rejectAgentAction(id),
  createAgentRule: vi.fn(),
  updateAgentRule: vi.fn(),
  deleteAgentRule: vi.fn(),
  createAgentPanel: vi.fn(),
  updateAgentPanel: vi.fn(),
  deleteAgentPanel: vi.fn(),
  currentPlatform: () => 'macos',
  getDraft: (...a: unknown[]) => getDraft(...a),
  saveDraft: (...a: unknown[]) => saveDraft(...a),
  deleteDraft: (...a: unknown[]) => deleteDraft(...a),
  getAccountSignature: () => Promise.resolve(null),
  sendReply: vi.fn(),
  reviewAgentDraft: (id: string, outcome: string) => reviewAgentDraft(id, outcome),
}));

import { AgentView } from './AgentView';

let container: HTMLDivElement;
let root: Root;

function action(id: string, status: AgentAction['status']): AgentAction {
  return {
    id,
    runId: 'r-new',
    ruleId: 'rule',
    ruleName: 'Support',
    kind: status === 'pending' ? 'archive' : 'draftReply',
    detail: '',
    status,
    requiresApproval: status === 'pending',
    result: status === 'done' ? 'draft-9' : null,
    error: null,
    createdAt: 10,
    decidedAt: null,
    runTitle: 'Cannot log in',
    reviewOutcome: null,
    reviewedAt: null,
    needsReview: status === 'done',
  };
}

function run(id: string, title: string, createdAt: number, actions: AgentAction[] = []): AgentRun {
  return {
    id,
    accountId: 'acc',
    trigger: 'email',
    triggerRef: `e-${id}`,
    title,
    sender: 'Ana',
    status: 'matched',
    summary: `Summary of ${title}`,
    error: null,
    createdAt,
    actions,
  };
}

function overview(partial: Partial<AgentOverview> = {}): AgentOverview {
  const pending = action('a-pending', 'pending');
  const done = action('a-done', 'done');
  return {
    enabled: true,
    rules: [],
    panels: [{ id: 'p', title: 'Support today', prompt: 'support', window: 'today', createdAt: 1, count: 4 }],
    feed: [run('r-new', 'Cannot log in', 20, [pending, done]), run('r-old', 'Invoice question', 10)],
    actions: [pending, done],
    ...partial,
  };
}

const q = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);

async function render() {
  await act(async () => {
    root.render(<AgentView onOpenEmail={() => {}} />);
  });
}

async function click(el: HTMLElement | null) {
  await act(async () => {
    el?.click();
  });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getAgentOverview.mockResolvedValue(overview());
  setAgentEnabled.mockResolvedValue();
  approveAgentAction.mockResolvedValue();
  rejectAgentAction.mockResolvedValue();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe('AgentView', () => {
  it('shows the feed oldest first like a chat, the panels and the actions to review', async () => {
    await render();
    const runs = [...container.querySelectorAll('[data-testid^="agent-run-"]')].map((el) =>
      el.getAttribute('data-testid'),
    );
    expect(runs).toEqual(['agent-run-r-old', 'agent-run-r-new']);
    expect(container.textContent).toContain('Summary of Cannot log in');
    expect(q('agent-panel-p')?.textContent).toContain('4');
    // The pending archive and the draft waiting to be sent are both to review.
    expect(container.textContent).toContain('agent:actions.pending:2');
    expect(q('agent-approve-a-pending')).not.toBeNull();
    expect(q('agent-approve-a-done')).toBeNull();
    expect(q('agent-review-a-done')).not.toBeNull();
  });

  it('has no refresh button: the view follows the backend on its own', async () => {
    await render();
    expect(q('agent-refresh')).toBeNull();
  });

  it('approves and rejects a pending action, then reloads', async () => {
    await render();
    await click(q('agent-approve-a-pending'));
    expect(approveAgentAction).toHaveBeenCalledWith('a-pending');
    await click(q('agent-reject-a-pending'));
    expect(rejectAgentAction).toHaveBeenCalledWith('a-pending');
    expect(getAgentOverview).toHaveBeenCalledTimes(3);
  });

  it('a failed approval is shown and logged', async () => {
    approveAgentAction.mockRejectedValue(new Error('queue closed'));
    await render();
    await click(q('agent-approve-a-pending'));
    expect(q('agent-error')?.textContent).toContain('queue closed');
    expect(addLog).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('queue closed'));
  });

  it('turning the agent on persists it and the off notice goes away', async () => {
    getAgentOverview.mockResolvedValueOnce(overview({ enabled: false })).mockResolvedValue(overview());
    await render();
    expect(q('agent-off-notice')).not.toBeNull();
    await click(container.querySelector<HTMLElement>('[role="switch"]'));
    expect(setAgentEnabled).toHaveBeenCalledWith(true);
    expect(q('agent-off-notice')).toBeNull();
  });

  it('reloads when the backend says the agent changed', async () => {
    await render();
    getAgentOverview.mockResolvedValue(overview({ feed: [], actions: [] }));
    await act(async () => {
      agentUpdated?.();
    });
    expect(q('agent-run-r-new')).toBeNull();
    expect(q('agent-feed-empty')).not.toBeNull();
  });

  it('a draft is reviewed and sent from the view, then recorded as sent', async () => {
    getDraft.mockResolvedValue({
      id: 'draft-9',
      emailId: 'e-r-new',
      accountId: 'acc',
      toAddresses: ['ana@example.com'],
      ccAddresses: [],
      subject: 'Re: Cannot log in',
      body: 'Hi Ana, send a screenshot',
      bodyHtml: null,
      providerDraftId: null,
    });
    outboxSend.mockResolvedValue('queued');
    reviewAgentDraft.mockResolvedValue();
    await render();
    await click(q('agent-review-a-done'));
    expect(q('agent-review')).not.toBeNull();
    expect(q('agent-run-r-new')).toBeNull();
    expect(q('preview-e-r-new')).not.toBeNull();
    const box = q('agent-review-draft') as HTMLTextAreaElement;
    expect(box.value).toBe('Hi Ana, send a screenshot');
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(
        box,
        'Hi Ana, a screenshot please',
      );
      box.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await click(q('agent-review-send'));
    expect(outboxSend).toHaveBeenCalledTimes(1);
    const [message, opts] = outboxSend.mock.calls[0];
    expect(message.body).toContain('a screenshot please');
    expect(message.replyToEmailId).toBe('e-r-new');
    expect(opts.draftId).toBe('draft-9');
    expect(reviewAgentDraft).toHaveBeenCalledWith('a-done', 'sent');
    expect(container.textContent).toContain('agent:review.sentNotice');
  });

  it('a draft can be discarded from the view', async () => {
    getDraft.mockResolvedValue({
      id: 'draft-9',
      emailId: 'e-r-new',
      accountId: 'acc',
      toAddresses: ['ana@example.com'],
      ccAddresses: [],
      subject: 'Re: x',
      body: 'Hi',
      bodyHtml: null,
      providerDraftId: null,
    });
    deleteDraft.mockResolvedValue(undefined);
    reviewAgentDraft.mockResolvedValue();
    await render();
    await click(q('agent-review-a-done'));
    await click(q('agent-review-discard'));
    expect(deleteDraft).toHaveBeenCalledWith('draft-9', 'acc');
    expect(reviewAgentDraft).toHaveBeenCalledWith('a-done', 'discarded');
    expect(outboxSend).not.toHaveBeenCalled();
  });

  it('a chip in the feed opens its action in the review pane, and back returns to the feed', async () => {
    await render();
    await click(q('agent-chip-a-pending'));
    expect(q('agent-review-approve')).not.toBeNull();
    await click(q('agent-review-approve'));
    expect(approveAgentAction).toHaveBeenCalledWith('a-pending');
    await click(
      [...container.querySelectorAll('button')].find((b) => b.textContent?.includes('agent:review.back')) ?? null,
    );
    expect(q('agent-run-r-new')).not.toBeNull();
  });

  it('an empty feed offers to create the first rule', async () => {
    getAgentOverview.mockResolvedValue(overview({ feed: [], actions: [] }));
    await render();
    await click(q('agent-feed-create-rule'));
    expect(document.querySelector('[data-testid="agent-rules-dialog"]')).not.toBeNull();
  });
});
