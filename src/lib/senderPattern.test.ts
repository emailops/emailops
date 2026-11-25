// The sender pattern of an attachment rule is a comma-separated list; its
// autocomplete works on the entry under the cursor.

import { describe, expect, it } from 'vitest';
import { replaceSenderEntry, senderEntryAt } from './senderPattern';

describe('senderEntryAt', () => {
  it.each([
    ['bill', 4, { start: 0, end: 4, text: 'bill' }],
    ['a@x.example, bil', 16, { start: 13, end: 16, text: 'bil' }],
    ['a@x.example,bil', 15, { start: 12, end: 15, text: 'bil' }],
    ['bil, a@x.example', 3, { start: 0, end: 3, text: 'bil' }],
    ['  bill  ', 6, { start: 2, end: 6, text: 'bill' }],
  ])('finds the entry of %j at %i', (value, cursor, entry) => {
    expect(senderEntryAt(value, cursor)).toEqual(entry);
  });

  it('drops the wildcards from the text it searches with', () => {
    expect(senderEntryAt('*apple*', 7).text).toBe('apple');
  });
});

describe('replaceSenderEntry', () => {
  it('puts the address in place of the entry and keeps the others', () => {
    const value = 'a@x.example, bil, c@y.example';
    expect(replaceSenderEntry(value, senderEntryAt(value, 16), 'billing@vendor.example')).toBe(
      'a@x.example, billing@vendor.example, c@y.example',
    );
  });

  it('replaces a lone entry', () => {
    expect(replaceSenderEntry('bil', senderEntryAt('bil', 3), 'billing@vendor.example')).toBe('billing@vendor.example');
  });
});
