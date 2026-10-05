import { describe, expect, it } from 'vitest';
import { mailProviderName } from './providerNames';

describe('mailProviderName', () => {
  it.each([
    ['gmail', 'Gmail'],
    ['outlook', 'Outlook'],
    ['imap', 'IMAP'],
  ])('%s is shown as %s', (provider, name) => {
    expect(mailProviderName(provider)).toBe(name);
  });

  it('shows an unknown provider as it came', () => {
    expect(mailProviderName('jmap')).toBe('jmap');
  });
});
