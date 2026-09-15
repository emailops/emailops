import { describe, expect, test } from 'vitest';

import type { Email } from '@/types';

import { forwardQuote, forwardSubject, loadForwardBody } from './forward';

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
    // Sync stores the display name and the address in separate fields; the
    // name never carries the address.
    sender: 'Bookings',
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

  /// Forwarding exists to pass the message on — headers alone tell the
  /// recipient that something was sent, not what it said.
  test('carries the original message below the headers', () => {
    const quote = forwardQuote(email({ body: '<div>Hi there,<br><br>The booking is attached.</div>' }), LABELS, fmt);
    expect(quote).toContain('Hi there,\n\nThe booking is attached.');
    expect(quote.indexOf('The booking is attached.')).toBeGreaterThan(quote.indexOf('To: me@example.test'));
  });
});

/// A thread loads without bodies — only the selected message has one — so the
/// message being forwarded may still need its body fetched.
describe('loadForwardBody', () => {
  test('uses the body the thread already has', async () => {
    const body = await loadForwardBody(
      email({ body: '<p>already here</p>' }),
      async () => '<p>fetched</p>',
      () => {},
    );
    expect(body).toBe('<p>already here</p>');
  });

  test('fetches the body when the thread arrived without it', async () => {
    const body = await loadForwardBody(
      email({ body: '' }),
      async () => '<p>fetched</p>',
      () => {},
    );
    expect(body).toBe('<p>fetched</p>');
  });

  /// The attachments still travel, so a forward without the original text is
  /// worth sending — but the user has to be told why the text is missing.
  test('degrades to no body, and reports why, when the fetch fails', async () => {
    const errors: unknown[] = [];
    const body = await loadForwardBody(
      email({ body: '' }),
      async () => {
        throw new Error('database is locked');
      },
      (err) => errors.push(err),
    );
    expect(body).toBe('');
    expect(errors).toHaveLength(1);
  });
});
