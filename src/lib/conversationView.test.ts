import { describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import { CONVERSATION_VIEW_PREF, parseConversationView, planOwnFirst } from './conversationView';

const ACCOUNT = 'acc1';
const ME = 'me@example.com';

function message(id: string, timestamp: number, overrides: Partial<Email> = {}): Email {
  return {
    id,
    accountId: ACCOUNT,
    threadId: 'thread-1',
    messageId: id,
    subject: 'Quarterly plan',
    sender: 'Alice',
    senderEmail: 'alice@example.com',
    recipients: [ME],
    cc: [],
    body: '',
    snippet: '',
    timestamp,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
    isStarred: false,
    ...overrides,
  };
}

/** A message the user wrote, as the provider files it (Sent, sent flag). */
function mine(id: string, timestamp: number, overrides: Partial<Email> = {}): Email {
  return message(id, timestamp, { sender: 'Me', senderEmail: ME, mailbox: 'sent', isSent: true, ...overrides });
}

const ids = (emails: Email[]) => emails.map((e) => e.id);

describe('parseConversationView', () => {
  it('reads the stored view', () => {
    expect(parseConversationView('own-first')).toBe('own-first');
    expect(parseConversationView('per-message')).toBe('per-message');
  });

  it('keeps the per-message view when nothing, or something unknown, is stored', () => {
    expect(parseConversationView(null)).toBe('per-message');
    expect(parseConversationView('')).toBe('per-message');
    expect(parseConversationView('newest-first')).toBe('per-message');
  });

  it('is stored under its own UI preference', () => {
    expect(CONVERSATION_VIEW_PREF).toBe('ui.conversation_view');
  });
});

describe('planOwnFirst', () => {
  it('puts the user’s latest message first and the rest oldest first', () => {
    const thread = [message('a', 1), mine('b', 2), message('c', 3), message('d', 4)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own?.id).toBe('b');
    expect(ids(plan.rest)).toEqual(['a', 'c', 'd']);
  });

  it('picks the most recent of several own messages and keeps the earlier ones in place', () => {
    const thread = [mine('a', 1), message('b', 2), mine('c', 3), message('d', 4), mine('e', 5), message('f', 6)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own?.id).toBe('e');
    expect(ids(plan.rest)).toEqual(['a', 'b', 'c', 'd', 'f']);
  });

  it('falls back to the whole conversation, oldest first, when the user never wrote in it', () => {
    const thread = [message('a', 1), message('b', 2), message('c', 3)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own).toBeNull();
    expect(ids(plan.rest)).toEqual(['a', 'b', 'c']);
  });

  it('shows a single own message on its own', () => {
    const plan = planOwnFirst([mine('a', 1)], ACCOUNT, ME);
    expect(plan.own?.id).toBe('a');
    expect(plan.rest).toEqual([]);
  });

  it('shows a single received message through the fallback', () => {
    const plan = planOwnFirst([message('a', 1)], ACCOUNT, ME);
    expect(plan.own).toBeNull();
    expect(ids(plan.rest)).toEqual(['a']);
  });

  it('returns nothing for an empty conversation', () => {
    expect(planOwnFirst([], ACCOUNT, ME)).toEqual({ own: null, rest: [] });
  });

  it('orders by date even when the messages arrive out of order', () => {
    const thread = [message('c', 3), mine('b', 2), message('a', 1)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own?.id).toBe('b');
    expect(ids(plan.rest)).toEqual(['a', 'c']);
  });

  it('breaks a tie on the date by the conversation’s own order', () => {
    const thread = [message('a', 1), mine('b', 5), mine('c', 5), message('d', 5), message('e', 5)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own?.id).toBe('c');
    expect(ids(plan.rest)).toEqual(['a', 'b', 'd', 'e']);
  });

  it('shows a message stored twice only once', () => {
    const thread = [message('a', 1), mine('b', 2), message('a', 1), mine('b', 2), message('c', 3)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own?.id).toBe('b');
    expect(ids(plan.rest)).toEqual(['a', 'c']);
  });

  it('leaves out messages of another account, own or not', () => {
    const thread = [
      message('a', 1),
      mine('x', 2, { accountId: 'acc2' }),
      message('b', 3),
      message('y', 4, { accountId: 'acc2' }),
    ];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    expect(plan.own).toBeNull();
    expect(ids(plan.rest)).toEqual(['a', 'b']);
  });

  it('shows every message of the conversation exactly once', () => {
    const thread = [message('a', 1), mine('b', 2), message('c', 2), mine('d', 3), message('e', 4)];
    const plan = planOwnFirst(thread, ACCOUNT, ME);
    const shown = [...(plan.own ? [plan.own] : []), ...plan.rest];
    expect(ids(shown).sort()).toEqual(['a', 'b', 'c', 'd', 'e']);
  });

  describe('which messages are the user’s own (the inbox rows’ rule, isOwnMessage)', () => {
    it('counts the provider’s sent flag, which knows aliases and send-as identities', () => {
      const thread = [
        message('a', 1),
        message('b', 2, { senderEmail: 'alias@example.org', mailbox: 'inbox', isSent: true }),
      ];
      expect(planOwnFirst(thread, ACCOUNT, ME).own?.id).toBe('b');
    });

    it('counts a message filed in Sent', () => {
      const thread = [message('a', 1), message('b', 2, { senderEmail: ME, mailbox: 'sent', isSent: false })];
      expect(planOwnFirst(thread, ACCOUNT, ME).own?.id).toBe('b');
    });

    it('counts an archived message from the account’s address', () => {
      const thread = [message('a', 1), message('b', 2, { senderEmail: 'Me@Example.com', mailbox: 'archive' })];
      expect(planOwnFirst(thread, ACCOUNT, ME).own?.id).toBe('b');
    });

    it('does not trust a From header equal to the account’s address in the inbox', () => {
      const thread = [message('a', 1), message('b', 2, { senderEmail: ME, mailbox: 'inbox', isSent: false })];
      expect(planOwnFirst(thread, ACCOUNT, ME).own).toBeNull();
    });

    it('relies on the sent flag and Sent alone when the account’s address is unknown', () => {
      const thread = [message('a', 1, { senderEmail: ME, mailbox: 'archive' }), mine('b', 2)];
      expect(planOwnFirst(thread, ACCOUNT, undefined).own?.id).toBe('b');
      expect(planOwnFirst([thread[0]], ACCOUNT, undefined).own).toBeNull();
    });
  });
});
