import { describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import { buildOccurrenceSlots, getThreadSearchMatches, stepMatchIndex } from './threadSearch';

function mkEmail(overrides: Partial<Email> & { id: string }): Email {
  return {
    accountId: 'acc-1',
    threadId: 'th-1',
    messageId: null,
    subject: '',
    sender: '',
    senderEmail: '',
    recipients: [],
    cc: [],
    body: '',
    snippet: '',
    timestamp: 0,
    isRead: true,
    triageStatus: null,
    category: 'primary' as Email['category'],
    mailbox: 'inbox',
    ...overrides,
  };
}

describe('getThreadSearchMatches', () => {
  it('returns no matches for an empty or whitespace-only query', () => {
    const emails = [mkEmail({ id: 'a', subject: 'Quarterly report' })];
    expect(getThreadSearchMatches(emails, '')).toEqual([]);
    expect(getThreadSearchMatches(emails, '   ')).toEqual([]);
  });

  it('matches the subject case-insensitively', () => {
    const emails = [mkEmail({ id: 'a', subject: 'Quarterly REPORT' }), mkEmail({ id: 'b', subject: 'Lunch plans' })];
    expect(getThreadSearchMatches(emails, 'report')).toEqual(['a']);
  });

  it('matches the snippet', () => {
    const emails = [mkEmail({ id: 'a', snippet: 'see the attached invoice for details' })];
    expect(getThreadSearchMatches(emails, 'Invoice')).toEqual(['a']);
  });

  it('matches sender name and sender email', () => {
    const emails = [
      mkEmail({ id: 'a', sender: 'Dana Smith', senderEmail: 'dana@example.com' }),
      mkEmail({ id: 'b', sender: 'Bob', senderEmail: 'bob@other.org' }),
    ];
    expect(getThreadSearchMatches(emails, 'dana')).toEqual(['a']);
    expect(getThreadSearchMatches(emails, 'other.org')).toEqual(['b']);
  });

  it('matches visible body text but not HTML markup', () => {
    const emails = [mkEmail({ id: 'a', body: '<div class="quarterly"><p>hello world</p></div>' })];
    expect(getThreadSearchMatches(emails, 'hello world')).toEqual(['a']);
    // "quarterly" only appears inside a tag attribute — the user can't see it,
    // and the in-frame highlighter would find nothing to mark.
    expect(getThreadSearchMatches(emails, 'quarterly')).toEqual([]);
  });

  it('ignores style and script blocks in the body', () => {
    const emails = [mkEmail({ id: 'a', body: '<style>.zebra { color: red; }</style><p>giraffe</p>' })];
    expect(getThreadSearchMatches(emails, 'zebra')).toEqual([]);
    expect(getThreadSearchMatches(emails, 'giraffe')).toEqual(['a']);
  });

  it('matches text split only by tags as separate words', () => {
    // "<p>foo</p><p>bar</p>" must not match the query "foobar" — the rendered
    // text has a break between the paragraphs.
    const emails = [mkEmail({ id: 'a', body: '<p>foo</p><p>bar</p>' })];
    expect(getThreadSearchMatches(emails, 'foobar')).toEqual([]);
  });

  it('preserves thread order in the returned ids', () => {
    const emails = [
      mkEmail({ id: 'first', subject: 'budget v1' }),
      mkEmail({ id: 'second', snippet: 'no relation' }),
      mkEmail({ id: 'third', body: 'final budget attached' }),
    ];
    expect(getThreadSearchMatches(emails, 'budget')).toEqual(['first', 'third']);
  });
});

describe('buildOccurrenceSlots', () => {
  it('gives one slot per matching email while body counts are unknown', () => {
    expect(buildOccurrenceSlots(['a', 'b'], {})).toEqual([
      { emailId: 'a', indexInEmail: 0 },
      { emailId: 'b', indexInEmail: 0 },
    ]);
  });

  it('expands to one slot per in-body occurrence once counts are reported', () => {
    expect(buildOccurrenceSlots(['a', 'b'], { a: 3, b: 2 })).toEqual([
      { emailId: 'a', indexInEmail: 0 },
      { emailId: 'a', indexInEmail: 1 },
      { emailId: 'a', indexInEmail: 2 },
      { emailId: 'b', indexInEmail: 0 },
      { emailId: 'b', indexInEmail: 1 },
    ]);
  });

  it('keeps one slot for a matching email with zero in-body occurrences (subject/sender match)', () => {
    expect(buildOccurrenceSlots(['a'], { a: 0 })).toEqual([{ emailId: 'a', indexInEmail: 0 }]);
  });

  it('returns no slots when nothing matches', () => {
    expect(buildOccurrenceSlots([], { a: 4 })).toEqual([]);
  });
});

describe('stepMatchIndex', () => {
  it('advances to the next match', () => {
    expect(stepMatchIndex(0, 1, 3)).toBe(1);
  });

  it('wraps from the last match to the first', () => {
    expect(stepMatchIndex(2, 1, 3)).toBe(0);
  });

  it('wraps from the first match back to the last', () => {
    expect(stepMatchIndex(0, -1, 3)).toBe(2);
  });

  it('returns -1 when there are no matches', () => {
    expect(stepMatchIndex(0, 1, 0)).toBe(-1);
  });
});
