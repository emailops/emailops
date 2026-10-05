import { describe, expect, it } from 'vitest';
import type { AgentAction, AgentRun } from '@/types';
import { buildReplyMessage, chronological, splitActions } from './agentFeed';

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
    reviewOutcome: null,
    reviewedAt: null,
    needsReview: false,
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

  it('a draft still waiting for review sits with the actions to review', () => {
    const draft = { ...action('d', 'done'), kind: 'draftReply' as const, needsReview: true };
    const { pending, recent } = splitActions([action('p', 'pending'), draft, action('r', 'rejected')]);
    expect(pending.map((a) => a.id)).toEqual(['p', 'd']);
    expect(recent.map((a) => a.id)).toEqual(['r']);
  });
});

describe('buildReplyMessage', () => {
  const draft = { toAddresses: ['laura@example.com'], ccAddresses: ['team@example.com'] };

  it('replies to the email with the reviewed text, to the draft recipients', () => {
    const m = buildReplyMessage({
      accountId: 'acc',
      emailId: 'e1',
      draft,
      text: 'Hola Laura\n\nGracias',
      signature: null,
    });
    expect(m.accountId).toBe('acc');
    expect(m.replyToEmailId).toBe('e1');
    expect(m.to).toEqual(['laura@example.com']);
    expect(m.cc).toEqual(['team@example.com']);
    expect(m.subject).toBe('');
    expect(m.body).toContain('Hola Laura');
    expect(m.body).toContain('Gracias');
    expect(m.bodyHtml).toContain('Hola Laura');
    expect(m.attachments).toEqual([]);
  });

  it('adds the account signature when it is on for replies, and only then', () => {
    const signature = {
      accountId: 'acc',
      html: '<p>Ulises · Demo</p>',
      useForNew: true,
      useForReplies: true,
      updatedAt: null,
    };
    const signed = buildReplyMessage({ accountId: 'acc', emailId: 'e1', draft, text: 'Hola', signature });
    expect(signed.bodyHtml).toContain('Ulises · Demo');
    expect(signed.body).toContain('Ulises · Demo');
    const off = buildReplyMessage({
      accountId: 'acc',
      emailId: 'e1',
      draft,
      text: 'Hola',
      signature: { ...signature, useForReplies: false },
    });
    expect(off.bodyHtml).not.toContain('Ulises · Demo');
  });
});
