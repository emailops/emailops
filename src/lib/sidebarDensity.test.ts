import { describe, expect, it } from 'vitest';
import { isSidebarDensity, type SidebarDensityClasses, sidebarDensityClasses } from './sidebarDensity';

describe('isSidebarDensity', () => {
  it.each(['comfortable', 'compact'])('accepts %s', (value) => {
    expect(isSidebarDensity(value)).toBe(true);
  });

  it.each([
    ['an unknown level', 'spacious'],
    ['an empty string', ''],
    ['the JSON-encoded form', '"compact"'],
    ['a different case', 'Compact'],
    ['a non-string', 1],
    ['null', null],
  ])('rejects %s', (_label, value) => {
    expect(isSidebarDensity(value)).toBe(false);
  });
});

describe('sidebarDensityClasses', () => {
  it('gives comfortable exactly the classes the sidebar uses today', () => {
    const today: SidebarDensityClasses = {
      nav: 'p-4 space-y-6',
      sectionHeader: 'mb-2',
      list: 'space-y-1',
      viewRow: 'py-2',
      accountRow: 'py-1.5',
      filterRow: 'py-1.5',
      tagGroup: 'mt-3',
    };
    expect(sidebarDensityClasses('comfortable')).toEqual(today);
  });

  it('gives compact tighter vertical spacing only', () => {
    const compact: SidebarDensityClasses = {
      nav: 'px-4 py-2 space-y-3',
      sectionHeader: 'mb-1.5',
      list: 'space-y-0.5',
      viewRow: 'py-1',
      // WebKit lays the account row's truncated text-xs label out 1 px short
      // (15 px for a 16 px line height), so py-1 alone gives 23 px; the floor
      // keeps the row at the 24 px target size.
      accountRow: 'py-1 min-h-6',
      filterRow: 'py-0.5',
      tagGroup: 'mt-1.5',
    };
    expect(sidebarDensityClasses('compact')).toEqual(compact);
  });
});
