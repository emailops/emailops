import { describe, expect, it } from 'vitest';
import { planViewChange } from './viewNavigation';

describe('planViewChange', () => {
  it('resets inbox filters only when switching to the inbox view', () => {
    expect(planViewChange('inbox', 'split').resetInboxFilters).toBe(true);
    expect(planViewChange('sent', 'split').resetInboxFilters).toBe(false);
    expect(planViewChange('contacts', 'split').resetInboxFilters).toBe(false);
  });

  it('closes the open email when switching views in full-width layout', () => {
    expect(planViewChange('sent', 'full-width').closeOpenEmail).toBe(true);
    expect(planViewChange('dashboard', 'full-width').closeOpenEmail).toBe(true);
    expect(planViewChange('inbox', 'full-width').closeOpenEmail).toBe(true);
  });

  it('keeps the open email when switching views in split layout', () => {
    expect(planViewChange('sent', 'split').closeOpenEmail).toBe(false);
    expect(planViewChange('inbox', 'split').closeOpenEmail).toBe(false);
  });
});
