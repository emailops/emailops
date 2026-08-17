import { describe, expect, test } from 'vitest';

import type { Email } from '@/types';

import { forwardQuote, forwardSubject } from './forward';

const LABELS = {
  header: 'Forwarded message',
  from: 'From',
  date: 'Date',
  subject: 'Subject',
  to: 'To',
  cc: 'Cc',
};

function email(overrides: Partial<Email> = {}): Email {
  return {
    id: 'e1',
    accountId: 'a1',
    threadId: 't1',
    messageId: '<m1@example.test>',
    subject: 'Booking confirmed',
    sender: 'Bookings <bookings@example.test>',
    senderEmail: 'bookings@example.test',
    recipients: ['me@example.test'],
    cc: [],
    body: 'body',
    snippet: 'body',
    timestamp: 1_700_000_000,
    isRead: true,
    triageStatus: null,
    category: 'primary',
    mailbox: 'inbox',
    isSent: false,
    ...overrides,
  };
}

describe('forwardSubject', () => {
  test('prefixes the subject', () => {
    expect(forwardSubject('Booking confirmed')).toBe('Fwd: Booking confirmed');
  });

  /// Forwarding on something that was forwarded to you is the common case —
  /// stacking prefixes is how a subject line turns into noise.
  test('does not stack prefixes, whatever client wrote the first one', () => {
    for (const already of ['Fwd: Trip', 'FW: Trip', 'RV: Trip', 'Wg: Trip', 'tr: Trip']) {
      expect(forwardSubject(already)).toBe(already);
    }
  });

  test('survives an empty subject', () => {
    expect(forwardSubject('   ')).toBe('Fwd:');
  });
});

describe('forwardQuote', () => {
  const fmt = (ts: number) => `date(${ts})`;

  test('carries the headers the recipient needs to make sense of it', () => {
    const quote = forwardQuote(email(), LABELS, fmt);
    expect(quote).toContain('---------- Forwarded message ----------');
    expect(quote).toContain('From: Bookings <bookings@example.test>');
    expect(quote).toContain('Date: date(1700000000)');
    expect(quote).toContain('Subject: Booking confirmed');
    expect(quote).toContain('To: me@example.test');
  });

  /// An empty "Cc:" line reads as a bug in the forwarding, not as an absence.
  test('omits Cc when there was none', () => {
    expect(forwardQuote(email(), LABELS, fmt)).not.toContain('Cc:');
    expect(forwardQuote(email({ cc: ['other@example.test'] }), LABELS, fmt)).toContain('Cc: other@example.test');
  });

  /// The body is composed above the quote, so the quote has to start with the
  /// blank lines that leave room for it.
  test('opens with room to write above it', () => {
    expect(forwardQuote(email(), LABELS, fmt).startsWith('\n\n')).toBe(true);
  });

  test('falls back to the bare address when there is no display name', () => {
    const quote = forwardQuote(email({ sender: '' }), LABELS, fmt);
    expect(quote).toContain('From: bookings@example.test');
  });
});
