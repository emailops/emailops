/**
 * Tests for Reply All recipient computation.
 *
 * Originally written as `console.assert` blocks before vitest was wired in.
 * Now expressed as proper vitest cases so `npm run test:frontend` picks them up.
 */

import { describe, expect, test } from 'vitest';
import type { Account, Email } from '@/types';

// -- Extracted logic under test --

function extractEmail(raw: string): string {
  const match = raw.match(/<([^>]+)>/);
  return (match ? match[1] : raw).trim().toLowerCase();
}

function computeReplyAllRecipients(email: Email, threadEmails: Email[], selfEmails: string[]): string[] {
  const all = new Set<string>();
  for (const msg of threadEmails) {
    all.add(extractEmail(msg.senderEmail));
    for (const r of msg.recipients) {
      const clean = extractEmail(r);
      if (clean.includes('@')) all.add(clean);
    }
  }
  // Also include the latest email's sender (may not be in thread if thread is empty)
  all.add(extractEmail(email.senderEmail));
  for (const r of email.recipients) {
    const clean = extractEmail(r);
    if (clean.includes('@')) all.add(clean);
  }
  // Remove self
  for (const self of selfEmails) {
    all.delete(self);
  }
  return [...all];
}

// -- Test data helpers --

const selfEmail = 'me@mycompany.com';
const accounts: Account[] = [
  {
    id: 'acc1',
    provider: 'gmail',
    email: selfEmail,
    name: 'Me',
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
  },
];
const selfEmails = accounts.map((a) => a.email.toLowerCase());

function makeEmail(overrides: Partial<Email>): Email {
  return {
    id: 'e1',
    accountId: 'acc1',
    threadId: 't1',
    messageId: null,
    subject: 'Test',
    sender: 'Sender',
    senderEmail: 'sender@example.com',
    recipients: [selfEmail],
    cc: [],
    body: '',
    snippet: '',
    timestamp: 1000,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    ...overrides,
  };
}

describe('computeReplyAllRecipients', () => {
  test('includes both thread participants and excludes self', () => {
    const thread = [
      makeEmail({
        id: 'e1',
        senderEmail: 'alice@example.com',
        sender: 'Alice',
        recipients: [selfEmail, 'bob@example.com'],
        timestamp: 1000,
      }),
      makeEmail({
        id: 'e2',
        senderEmail: 'bob@example.com',
        sender: 'Bob',
        recipients: [selfEmail],
        timestamp: 2000,
      }),
    ];
    const latest = thread[thread.length - 1];
    const result = computeReplyAllRecipients(latest, thread, selfEmails);
    expect(result).toEqual(expect.arrayContaining(['alice@example.com', 'bob@example.com']));
    expect(result).not.toContain(selfEmail);
    expect(result).toHaveLength(2);
  });

  test('parses "Name <email>" format recipients', () => {
    const thread = [
      makeEmail({
        id: 'e1',
        senderEmail: 'alice@example.com',
        sender: 'Alice',
        recipients: [`Me <${selfEmail}>`, 'Bob <bob@example.com>'],
        timestamp: 1000,
      }),
    ];
    const result = computeReplyAllRecipients(thread[0], thread, selfEmails);
    expect(result).toEqual(expect.arrayContaining(['alice@example.com', 'bob@example.com']));
    expect(result).not.toContain(selfEmail);
  });

  test('deduplicates participants across thread messages', () => {
    const thread = [
      makeEmail({
        id: 'e1',
        senderEmail: 'alice@example.com',
        recipients: [selfEmail, 'bob@example.com'],
        timestamp: 1000,
      }),
      makeEmail({
        id: 'e2',
        senderEmail: 'alice@example.com',
        recipients: [selfEmail, 'bob@example.com'],
        timestamp: 2000,
      }),
    ];
    const result = computeReplyAllRecipients(thread[1], thread, selfEmails);
    expect(result).toHaveLength(2);
  });

  test('collects 3+ participants across a multi-message thread', () => {
    const thread = [
      makeEmail({
        id: 'e1',
        senderEmail: 'alice@a.com',
        recipients: [selfEmail],
        timestamp: 1000,
      }),
      makeEmail({
        id: 'e2',
        senderEmail: 'bob@b.com',
        recipients: [selfEmail, 'alice@a.com'],
        timestamp: 2000,
      }),
      makeEmail({
        id: 'e3',
        senderEmail: 'charlie@c.com',
        recipients: [selfEmail],
        timestamp: 3000,
      }),
    ];
    const result = computeReplyAllRecipients(thread[2], thread, selfEmails);
    expect(result).toEqual(expect.arrayContaining(['alice@a.com', 'bob@b.com', 'charlie@c.com']));
    expect(result).toHaveLength(3);
  });

  test('handles a single-message thread with multiple cc recipients', () => {
    const thread = [
      makeEmail({
        id: 'e1',
        senderEmail: 'alice@example.com',
        recipients: [selfEmail, 'bob@example.com', 'charlie@example.com'],
        timestamp: 1000,
      }),
    ];
    const result = computeReplyAllRecipients(thread[0], thread, selfEmails);
    expect(result).toHaveLength(3);
  });
});
