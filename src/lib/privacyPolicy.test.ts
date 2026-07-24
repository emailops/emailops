import { describe, expect, it } from 'vitest';

import { privacyPolicyUrl } from './privacyPolicy';

describe('privacyPolicyUrl', () => {
  it.each([
    ['en', 'https://getemailops.com/en/privacy/'],
    ['es', 'https://getemailops.com/privacy/'],
    ['fr', 'https://getemailops.com/fr/privacy/'],
    ['de', 'https://getemailops.com/de/privacy/'],
  ] as const)('maps %s to its localized policy page', (language, url) => {
    expect(privacyPolicyUrl(language)).toBe(url);
  });
});
