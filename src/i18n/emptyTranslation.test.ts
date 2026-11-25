import { describe, expect, it } from 'vitest';

import { initI18n } from './index';

// Some strings are empty on purpose: they mean "show nothing here". The junk
// banner shows `inbox:junk.kind.<kind>`, and a message the user marked as junk
// can carry the `legit` kind, whose label is ''.
describe('initI18n', () => {
  it('renders an empty translation as nothing, never as its key', async () => {
    const i18n = await initI18n('en');

    expect(i18n.t('inbox:junk.kind.legit')).toBe('');
  });
});
