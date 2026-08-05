import { describe, expect, it } from 'vitest';
import { shouldShowCategoryTabs, viewTitleKey } from './viewChrome';

describe('viewTitleKey', () => {
  it('titles each built-in view from the sidebar labels', () => {
    // Same key the navigation entry uses, so tapping "Calendar" cannot land on
    // a screen titled anything else.
    const cases: Array<[string, string]> = [
      ['inbox', 'sidebar:inbox'],
      ['calendar', 'sidebar:calendar'],
      ['chat', 'sidebar:chat'],
      ['drafts', 'sidebar:drafts'],
      ['dashboard', 'sidebar:dashboard'],
    ];
    for (const [mode, key] of cases) {
      expect(viewTitleKey(mode)).toBe(key);
    }
  });

  it('returns null for a user-created folder, whose name is not in any locale', () => {
    expect(viewTitleKey('folder:INBOX/Clients')).toBeNull();
    expect(viewTitleKey('folder:')).toBeNull();
  });

  it('returns null rather than guessing for an unknown view', () => {
    expect(viewTitleKey('')).toBeNull();
    expect(viewTitleKey('somethingNew')).toBeNull();
  });

  it('is not fooled by inherited Object properties', () => {
    // A bare lookup on an object literal would resolve 'constructor' to a
    // function and render "[object Function]" as a title.
    expect(viewTitleKey('constructor')).toBeNull();
    expect(viewTitleKey('toString')).toBeNull();
  });
});

describe('shouldShowCategoryTabs', () => {
  it('shows the strip once there is something to switch between', () => {
    expect(shouldShowCategoryTabs(true, 2)).toBe(true);
    expect(shouldShowCategoryTabs(true, 5)).toBe(true);
  });

  it('hides the strip for a single category', () => {
    // An account syncing only Primary got a one-tab strip that could not
    // change anything — a whole row restating the view's own name.
    expect(shouldShowCategoryTabs(true, 1)).toBe(false);
  });

  it('hides the strip when no categories are known yet', () => {
    // Account settings have not resolved; rendering nothing beats flashing tabs.
    expect(shouldShowCategoryTabs(true, 0)).toBe(false);
  });

  it('stays hidden for providers without categories at all', () => {
    // IMAP: the parent passes showCategoryFilter=false.
    expect(shouldShowCategoryTabs(false, 3)).toBe(false);
    expect(shouldShowCategoryTabs(false, 1)).toBe(false);
  });
});
