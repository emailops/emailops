// The "hide junk" checkbox above the inbox list must not show a count.
//
// It used to read "Hide {{count}} junk messages", where the count was the
// flagged messages among the *loaded* rows. The inbox loads incrementally, so
// scrolling made the number climb — a label that changes while the user reads it
// describes nothing, and reads as if junk were arriving in real time.
//
// A string with no `count` placeholder cannot render a number no matter what the
// caller passes in options, so pinning the strings pins the behaviour.

import { describe, expect, it } from 'vitest';

import { resources, SUPPORTED_LANGUAGES } from '@/i18n/resources';

describe('inbox junk.hideFlagged label', () => {
  it.each(SUPPORTED_LANGUAGES)('%s states the action without a count', (lang) => {
    const label = resources[lang].inbox.junk.hideFlagged;

    expect(typeof label).toBe('string');
    expect(label).not.toContain('{{count}}');
    // No stray placeholder of any name either — an unresolved `{{…}}` renders
    // literally, which is worse than the number it replaced.
    expect(label).not.toMatch(/\{\{.*?\}\}/);
  });
});
