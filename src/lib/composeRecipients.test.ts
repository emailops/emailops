import { describe, expect, it } from 'vitest';
import { extractEmail, mergePendingRecipient } from './composeRecipients';

describe('extractEmail', () => {
  it('strips angle brackets and normalizes case/whitespace', () => {
    expect(extractEmail('  Alice <Alice@Example.com> ')).toBe('alice@example.com');
  });

  it('returns a bare address lowercased and trimmed', () => {
    expect(extractEmail('  GERO@emailops.com ')).toBe('gero@emailops.com');
  });
});

describe('mergePendingRecipient', () => {
  // Regression: a valid email typed into the To box but not yet tokenized
  // (user didn't press Enter/Tab or pick a suggestion) must still count, so
  // Send isn't disabled and the address isn't dropped on send.
  it('includes a valid email still sitting in the input box', () => {
    expect(mergePendingRecipient([], 'gero@emailops.com')).toEqual(['gero@emailops.com']);
  });

  it('returns the committed list unchanged when the input is empty', () => {
    expect(mergePendingRecipient(['a@b.com'], '')).toEqual(['a@b.com']);
    expect(mergePendingRecipient(['a@b.com'], '   ')).toEqual(['a@b.com']);
  });

  it('ignores input that is not a plausible address', () => {
    expect(mergePendingRecipient([], 'not-an-email')).toEqual([]);
  });

  it('does not duplicate an address already committed', () => {
    expect(mergePendingRecipient(['a@b.com'], 'a@b.com')).toEqual(['a@b.com']);
    expect(mergePendingRecipient(['a@b.com'], 'A@B.com')).toEqual(['a@b.com']);
  });

  it('normalizes the pending address the same way committed tokens are', () => {
    expect(mergePendingRecipient(['x@y.com'], 'Bob <Bob@Example.com>')).toEqual(['x@y.com', 'bob@example.com']);
  });
});
