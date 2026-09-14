import { describe, expect, it } from 'vitest';
import { getTagChipColor, tagHeaderBackground } from './TagChips';

describe('tagHeaderBackground', () => {
  it('returns only the background half of the chip colour', () => {
    // Block headers pair the tint with their own dark text, so the chip's
    // own low-contrast foreground must not come along.
    expect(tagHeaderBackground('topic', 'billing')).toBe('bg-amber-50');
    expect(tagHeaderBackground('priority', 'urgent')).toBe('bg-red-100');
  });

  it('never returns a text- class', () => {
    for (const [type, value] of [
      ['company', 'globex'],
      ['priority', 'low'],
      ['intent', 'request'],
      ['topic', 'anything'],
      ['unknown-type', 'x'],
    ]) {
      expect(tagHeaderBackground(type, value)).not.toMatch(/text-/);
    }
  });

  it('falls back to a neutral tint for an unmapped tag type', () => {
    expect(tagHeaderBackground('not-a-type', 'whatever')).toMatch(/^bg-/);
  });

  it('stays consistent with the chip colour it is derived from', () => {
    const chip = getTagChipColor('intent', 'approval');
    expect(chip.split(' ')).toContain(tagHeaderBackground('intent', 'approval'));
  });
});
