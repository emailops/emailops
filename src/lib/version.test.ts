// Sidebar version label: releases show the bare version; local/dev builds
// (commit not tagged v{version}) append the short sha so a screenshot or bug
// report pins the exact code that was running.

import { describe, expect, it } from 'vitest';
import { formatVersionLabel } from './version';

describe('formatVersionLabel', () => {
  it('shows just the version for release builds', () => {
    expect(formatVersionLabel({ version: '0.6.2', commit: '05ae613', isRelease: true })).toBe('v0.6.2');
  });

  it('appends the commit short sha for non-release builds', () => {
    expect(formatVersionLabel({ version: '0.6.2', commit: '05ae613', isRelease: false })).toBe('v0.6.2 (05ae613)');
  });

  it('degrades to the bare version when git metadata is unavailable', () => {
    expect(formatVersionLabel({ version: '0.6.2', commit: null, isRelease: false })).toBe('v0.6.2');
  });
});
