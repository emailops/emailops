import { describe, expect, it } from 'vitest';

import { validateSenderDomains, validateSenderEmails } from './scopeValidation';

describe('validateSenderDomains', () => {
  it('parses a clean list', () => {
    expect(validateSenderDomains(' Stripe.com, wise.com ')).toEqual({
      values: ['stripe.com', 'wise.com'],
      error: null,
    });
  });

  it('flags a full address with the domain it probably meant', () => {
    expect(validateSenderDomains('billing@stripe.com').error).toEqual({
      code: 'domainLooksLikeEmail',
      params: { value: 'billing@stripe.com', domain: 'stripe.com' },
    });
  });

  it('flags a bare word as an invalid domain', () => {
    expect(validateSenderDomains('gmail').error).toEqual({ code: 'invalidDomain', params: { value: 'gmail' } });
  });
});

describe('validateSenderEmails', () => {
  it('flags an entry that is not an address', () => {
    expect(validateSenderEmails('ok@example.com, nope').error).toEqual({
      code: 'invalidEmail',
      params: { value: 'nope' },
    });
  });
});
