import { describe, expect, it } from 'vitest';
import type { ChatMessageSource } from '@/types';
import { buildIdSearchQuery, collectReferencedEmailIds } from './referencedEmails';

function source(emailId: string, citationNumber: number): ChatMessageSource {
  return {
    citationNumber,
    emailId,
    relevanceScore: 1,
    subject: 's',
    sender: 'x',
    senderEmail: 'x@ex.com',
    timestamp: 0,
  };
}

describe('collectReferencedEmailIds', () => {
  it('returns the sources the answer cites, in order of appearance', () => {
    const ids = collectReferencedEmailIds({
      content: 'Second [2], then first [1] and [2] again.',
      sources: [source('a', 1), source('b', 2), source('retrieved-only', 3)],
    });
    expect(ids).toEqual(['b', 'a']);
  });

  it('keeps uncited sources out when the answer cites something', () => {
    const ids = collectReferencedEmailIds({
      content: 'See [one](email://b).',
      sources: [source('a', 1), source('b', 2)],
      referencedEmailIds: ['a', 'b'],
    });
    expect(ids).toEqual(['b']);
  });

  it('falls back to the message sources, in order, when the answer cites nothing', () => {
    const ids = collectReferencedEmailIds({
      content: 'Write to help@vendor.example.',
      sources: [source('b', 2), source('a', 1)],
    });
    expect(ids).toEqual(['a', 'b']);
  });

  it('adds allowlisted email:// links from the answer, without duplicates', () => {
    const ids = collectReferencedEmailIds({
      content: 'Cited [1]. See [one](email://a) and **[two](email://acc-1::42)**.',
      sources: [source('a', 1)],
      referencedEmailIds: ['a', 'acc-1::42', 'not-mentioned'],
    });
    expect(ids).toEqual(['a', 'acc-1::42']);
  });

  it('ignores email:// links outside the allowlist (hallucinated ids)', () => {
    const ids = collectReferencedEmailIds({
      content: '[ghost](email://made-up)',
      sources: [],
      referencedEmailIds: ['real'],
    });
    expect(ids).toEqual([]);
  });
});

describe('buildIdSearchQuery', () => {
  it('joins one id: operator per email', () => {
    expect(buildIdSearchQuery(['a', 'acc-1::42'])).toBe('id:a id:acc-1::42');
  });
});
