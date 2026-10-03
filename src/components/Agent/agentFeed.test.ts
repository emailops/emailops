import { describe, expect, it } from 'vitest';
import type { AgentAction, AgentRun } from '@/types';
import { chronological, splitActions } from './agentFeed';

function action(id: string, status: AgentAction['status']): AgentAction {
  return {
    id,
    runId: 'run',
    ruleId: null,
    ruleName: 'Support',
    kind: 'archive',
    detail: '',
    status,
    requiresApproval: status === 'pending',
    result: null,
    error: null,
    createdAt: 1,
    decidedAt: null,
    runTitle: 'Title',
  };
}

function run(id: string, createdAt: number): AgentRun {
  return {
    id,
    accountId: 'acc',
    trigger: 'email',
    triggerRef: `e-${id}`,
    title: id,
    sender: 'Ana',
    status: 'matched',
    summary: '',
    error: null,
    createdAt,
    actions: [],
  };
}

describe('agent feed helpers', () => {
  it('shows the feed oldest first, like a chat, without mutating the input', () => {
    const feed = [run('new', 3), run('mid', 2), run('old', 1)];
    expect(chronological(feed).map((r) => r.id)).toEqual(['old', 'mid', 'new']);
    expect(feed[0].id).toBe('new');
  });

  it('splits the actions into the review queue and the decided ones, keeping their order', () => {
    const { pending, recent } = splitActions([
      action('p1', 'pending'),
      action('p2', 'pending'),
      action('d', 'done'),
      action('f', 'failed'),
      action('r', 'rejected'),
    ]);
    expect(pending.map((a) => a.id)).toEqual(['p1', 'p2']);
    expect(recent.map((a) => a.id)).toEqual(['d', 'f', 'r']);
  });
});
