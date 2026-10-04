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
    expect(container.textContent).toContain('agent:actions.pending:1');
    expect(q('agent-approve-a-pending')).not.toBeNull();
    expect(q('agent-approve-a-done')).toBeNull();
    expect(q('draft-draft-9')).not.toBeNull();
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

  it('refresh reloads the feed', async () => {
    await render();
    await click(q('agent-refresh'));
    expect(getAgentOverview).toHaveBeenCalledTimes(2);
  });

  it('an empty feed offers to create the first rule', async () => {
    getAgentOverview.mockResolvedValue(overview({ feed: [], actions: [] }));
    await render();
    await click(q('agent-feed-create-rule'));
    expect(document.querySelector('[data-testid="agent-rules-dialog"]')).not.toBeNull();
  });
});
