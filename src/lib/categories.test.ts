import { describe, expect, it } from 'vitest';
import { DEFAULT_CATEGORIES, VALID_CATEGORIES } from './categories';

describe('inbox category defaults', () => {
  it('shows every category the account may be syncing', () => {
    // Regression: the default was a hard-coded ['primary', 'social', 'updates'].
    // An account configured to sync Promotions downloaded that mail and then
    // filtered it out of the inbox, because a default the user never chose
    // excluded it. A default cannot know what an account syncs, so it must not
    // exclude anything — `availableCategories` decides which tabs exist.
    for (const category of VALID_CATEGORIES) {
      expect(DEFAULT_CATEGORIES).toContain(category);
    }
  });

  it('only contains categories the app knows how to filter', () => {
    for (const category of DEFAULT_CATEGORIES) {
      expect(VALID_CATEGORIES.has(category)).toBe(true);
    }
  });
});
