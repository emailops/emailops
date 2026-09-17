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
  it('returns source ids in citation order', () => {
    const ids = collectReferencedEmailIds({ content: 'answer', sources: [source('a', 1), source('b', 2)] });
    expect(ids).toEqual(['a', 'b']);
  });

  it('adds allowlisted email:// links from the answer, without duplicates', () => {
    const ids = collectReferencedEmailIds({
      content: 'See [one](email://a) and **[two](email://acc-1::42)**.',
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
