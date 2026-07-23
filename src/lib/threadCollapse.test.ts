import { describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import { getThreadViewItems } from './threadCollapse';

function makeEmail(id: string): Email {
  return {
    id,
    accountId: 'acc1',
    threadId: 'thread-1',
    messageId: id,
    subject: 'Test',
    sender: 'Alice',
    senderEmail: 'alice@example.com',
    recipients: [],
    cc: [],
    body: '<p>body</p>',
    snippet: 'body',
    timestamp: 1000,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
  };
}

function makeThread(count: number): Email[] {
  return Array.from({ length: count }, (_, i) => makeEmail(`e${i + 1}`));
}

describe('getThreadViewItems', () => {
  it('returns all emails unchanged for a single-message thread', () => {
    const emails = makeThread(1);
    const items = getThreadViewItems(emails, false);
    expect(items).toHaveLength(1);
    expect(items[0].type).toBe('email');
  });

  it('returns all emails unchanged for threads with 3 or fewer messages', () => {
    for (const count of [1, 2, 3]) {
      const items = getThreadViewItems(makeThread(count), false);
      expect(items.every((i) => i.type === 'email')).toBe(true);
      expect(items).toHaveLength(count);
    }
  });

  it('collapses middle emails when thread has 4 or more messages and isExpanded=false', () => {
    const emails = makeThread(4);
    const items = getThreadViewItems(emails, false);

    // Expect: first, collapsed, last
    expect(items).toHaveLength(3);
    expect(items[0].type).toBe('email');
    expect(items[1].type).toBe('collapsed');
    expect(items[2].type).toBe('email');
  });

  it('shows the first email as the first item when collapsed', () => {
    const emails = makeThread(5);
    const items = getThreadViewItems(emails, false);
    const first = items[0];
    expect(first.type).toBe('email');
    if (first.type === 'email') {
      expect(first.email.id).toBe('e1');
    }
  });

  it('shows the last email as the last item when collapsed', () => {
    const emails = makeThread(5);
    const items = getThreadViewItems(emails, false);
    const last = items[items.length - 1];
    expect(last.type).toBe('email');
    if (last.type === 'email') {
      expect(last.email.id).toBe('e5');
    }
  });

  it('collapsed item contains the correct count and emailIds for middle messages', () => {
    const emails = makeThread(5);
    const items = getThreadViewItems(emails, false);
    const collapsed = items[1];
    expect(collapsed.type).toBe('collapsed');
    if (collapsed.type === 'collapsed') {
      expect(collapsed.count).toBe(3); // e2, e3, e4
      expect(collapsed.emailIds).toEqual(['e2', 'e3', 'e4']);
    }
  });

  it('shows all emails when isExpanded=true even for long threads', () => {
    const emails = makeThread(10);
    const items = getThreadViewItems(emails, true);
    expect(items).toHaveLength(10);
    expect(items.every((i) => i.type === 'email')).toBe(true);
  });

  it('includes correct email index in each email item', () => {
    const emails = makeThread(3);
    const items = getThreadViewItems(emails, false);
    items.forEach((item, i) => {
      expect(item.type).toBe('email');
      if (item.type === 'email') {
        expect(item.index).toBe(i);
      }
    });
  });

  it('collapsed item for exactly 4 messages has count 2', () => {
    const emails = makeThread(4);
    const items = getThreadViewItems(emails, false);
    const collapsed = items[1];
    expect(collapsed.type).toBe('collapsed');
    if (collapsed.type === 'collapsed') {
      expect(collapsed.count).toBe(2); // e2, e3
    }
  });
});
