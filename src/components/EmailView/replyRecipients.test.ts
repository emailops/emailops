import { describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import { computeReplyRecipients } from './ReplyCompose';

const ME = 'me@mine.test';

function email(overrides: Partial<Email>): Email {
  return {
    id: 'e1',
    accountId: 'acc',
    threadId: 't1',
    messageId: null,
    subject: 'Subject',
    sender: 'Someone',
    senderEmail: 'someone@example.test',
    recipients: [],
    cc: [],
    body: '',
    snippet: '',
    timestamp: 1000,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
    isStarred: false,
    ...overrides,
  };
}

describe('computeReplyRecipients', () => {
  it('replies to the sender of an inbound message', () => {
    const inbound = email({ senderEmail: 'alice@example.test', recipients: [ME] });
    expect(computeReplyRecipients(inbound, [inbound], [ME])).toEqual(['alice@example.test']);
  });

  it('replies to the recipients when the last message is my own', () => {
    // Regression: replying to a thread whose latest message I sent used to
    // address the reply to myself, because it always used the sender.
    const mine = email({ senderEmail: ME, recipients: ['alice@example.test'], isSent: true });
    expect(computeReplyRecipients(mine, [mine], [ME])).toEqual(['alice@example.test']);
  });

  it('keeps every recipient of my own message', () => {
    const mine = email({
      senderEmail: ME,
      recipients: ['alice@example.test', 'bob@example.test'],
      isSent: true,
      isStarred: false,
    });
    expect(computeReplyRecipients(mine, [mine], [ME])).toEqual(['alice@example.test', 'bob@example.test']);
  });

  it('never addresses a reply to myself', () => {
    const mine = email({ senderEmail: ME, recipients: [ME, 'alice@example.test'], isSent: true });
    expect(computeReplyRecipients(mine, [mine], [ME])).not.toContain(ME);
  });

  it('falls back to the last inbound sender when my message has no usable recipients', () => {
    // A self-addressed note in a real conversation: the thread still has a
    // correspondent, so use them rather than producing an empty To field.
    const inbound = email({ id: 'a', senderEmail: 'alice@example.test', recipients: [ME], timestamp: 100 });
    const mine = email({ id: 'b', senderEmail: ME, recipients: [ME], isSent: true, timestamp: 200 });
    expect(computeReplyRecipients(mine, [inbound, mine], [ME])).toEqual(['alice@example.test']);
  });

  it('strips display names from recipient headers', () => {
    const mine = email({ senderEmail: ME, recipients: ['Alice <alice@example.test>'], isSent: true });
    expect(computeReplyRecipients(mine, [mine], [ME])).toEqual(['alice@example.test']);
  });

  it('compares self addresses case-insensitively', () => {
    // Providers vary the casing of the From header between messages.
    const mine = email({ senderEmail: 'Me@Mine.TEST', recipients: ['alice@example.test'], isSent: true });
    expect(computeReplyRecipients(mine, [mine], [ME])).toEqual(['alice@example.test']);
  });

  it('addresses a note only ever sent to myself back to its recipients', () => {
    // As Gmail does. The prefilled To is visible before sending, and a reply
    // from another of my accounts to my own note is a real use.
    const mine = email({ senderEmail: ME, recipients: ['Me <Me@Mine.test>'], isSent: true });
    expect(computeReplyRecipients(mine, [mine], [ME])).toEqual(['me@mine.test']);
  });
});
