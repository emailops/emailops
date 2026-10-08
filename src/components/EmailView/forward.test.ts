import { describe, expect, test } from 'vitest';

import type { Email } from '@/types';

import { forwardQuote, forwardSubject, loadForwardTexts } from './forward';

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
    isStarred: false,
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
  const one = (e: Email, text = 'body') => forwardQuote([{ email: e, text }], LABELS, fmt);

  test('carries the headers the recipient needs to make sense of it', () => {
    const quote = one(email());
    expect(quote).toContain('---------- Forwarded message ----------');
    expect(quote).toContain('From: Bookings <bookings@example.test>');
    expect(quote).toContain('Date: date(1700000000)');
    expect(quote).toContain('Subject: Booking confirmed');
    expect(quote).toContain('To: me@example.test');
  });

  /// An empty "Cc:" line reads as a bug in the forwarding, not as an absence.
  test('omits Cc when there was none', () => {
    expect(one(email())).not.toContain('Cc:');
    expect(one(email({ cc: ['other@example.test'] }))).toContain('Cc: other@example.test');
  });

  /// The body is composed above the quote, so the quote has to start with the
  /// blank lines that leave room for it.
  test('opens with room to write above it', () => {
    expect(one(email()).startsWith('\n\n')).toBe(true);
  });

  test('falls back to the bare address when there is no display name', () => {
    expect(one(email({ sender: '' }))).toContain('From: bookings@example.test');
  });

  /// Forwarding exists to pass the message on — headers alone tell the
  /// recipient that something was sent, not what it said.
  test('carries the original message below the headers', () => {
    const quote = one(email(), 'Hi there,\n\nThe booking is attached.');
    expect(quote).toContain('Hi there,\n\nThe booking is attached.');
    expect(quote.indexOf('The booking is attached.')).toBeGreaterThan(quote.indexOf('To: me@example.test'));
  });

  /// Forwarding a conversation forwards all of it. The latest reply alone
  /// carries only the history its own client chose to quote — and a reply
  /// that quotes nothing cuts every message before it out of the forward.
  test('carries every message of the thread, oldest first, each under its own headers', () => {
    const quote = forwardQuote(
      [
        { email: email({ id: 'e1', sender: 'Ana', senderEmail: 'ana@example.test', timestamp: 1 }), text: 'First ask' },
        { email: email({ id: 'e2', sender: 'Me', senderEmail: 'me@example.test', timestamp: 2 }), text: 'My answer' },
        { email: email({ id: 'e3', sender: 'Ana', senderEmail: 'ana@example.test', timestamp: 3 }), text: 'Thanks' },
      ],
      LABELS,
      fmt,
    );
    expect(quote.split('---------- Forwarded message ----------')).toHaveLength(4);
    const at = (s: string) => quote.indexOf(s);
    expect(at('First ask')).toBeGreaterThan(at('Date: date(1)'));
    expect(at('Date: date(2)')).toBeGreaterThan(at('First ask'));
    expect(at('My answer')).toBeGreaterThan(at('Date: date(2)'));
    expect(at('Date: date(3)')).toBeGreaterThan(at('My answer'));
    expect(at('Thanks')).toBeGreaterThan(at('Date: date(3)'));
  });

  /// A reply that only quoted adds nothing, but who answered and when is
  /// still part of the conversation.
  test('keeps the headers of a message with no new text', () => {
    const quote = one(email(), '');
    expect(quote).toContain('From: Bookings <bookings@example.test>');
    expect(quote.trimEnd().endsWith('To: me@example.test')).toBe(true);
  });
});

/// The text of each message comes from the backend, which drops the history
/// an earlier message of the thread already holds.
describe('loadForwardTexts', () => {
  test('pairs each message of the thread with its text', async () => {
    const thread = [email({ id: 'e1' }), email({ id: 'e2' })];
    const pairs = await loadForwardTexts(
      thread,
      async () => [
        { emailId: 'e1', text: 'one' },
        { emailId: 'e2', text: 'two' },
      ],
      () => {},
    );
    expect(pairs.map((p) => [p.email.id, p.text])).toEqual([
      ['e1', 'one'],
      ['e2', 'two'],
    ]);
  });

  /// The thread on screen may have gained a message since; the forward
  /// follows what the user sees, with the headers when the text is unknown.
  test('keeps a message the backend did not return, without text', async () => {
    const pairs = await loadForwardTexts(
      [email({ id: 'e1' }), email({ id: 'e2' })],
      async () => [{ emailId: 'e1', text: 'one' }],
      () => {},
    );
    expect(pairs.map((p) => [p.email.id, p.text])).toEqual([
      ['e1', 'one'],
      ['e2', ''],
    ]);
  });

  /// The attachments still travel, so a forward without the original text is
  /// worth sending — but the user has to be told why the text is missing.
  test('degrades to headers only, and reports why, when the fetch fails', async () => {
    const errors: unknown[] = [];
    const pairs = await loadForwardTexts(
      [email()],
      async () => {
        throw new Error('database is locked');
      },
      (err) => errors.push(err),
    );
    expect(pairs.map((p) => p.text)).toEqual(['']);
    expect(errors).toHaveLength(1);
  });
});
