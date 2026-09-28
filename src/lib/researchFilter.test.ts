import { describe, expect, it } from 'vitest';
import { researchFilterParts } from './researchFilter';

describe('researchFilterParts', () => {
  it('lists the fields the gather ran in reading order, dates as dd/mm/yyyy', () => {
    expect(researchFilterParts({ until: '2026-09-28', since: '2026-03-28', from: 'reports@example.com' })).toEqual([
      { field: 'from', value: 'reports@example.com' },
      { field: 'since', value: '28/03/2026' },
      { field: 'until', value: '27/09/2026' },
    ]);
  });

  it('shows the exclusive end bound as the last day it includes', () => {
    expect(researchFilterParts({ until: '2026-01-01' })).toEqual([{ field: 'until', value: '31/12/2025' }]);
    expect(researchFilterParts({ until: '2024-03-01' })).toEqual([{ field: 'until', value: '29/02/2024' }]);
  });

  it('shows keywords and tags as they were searched', () => {
    expect(researchFilterParts({ query: 'INV-2041', intent: 'billing' })).toEqual([
      { field: 'query', value: 'INV-2041' },
      { field: 'intent', value: 'billing' },
    ]);
  });

  it('shows the unread flag only when it filters', () => {
    expect(researchFilterParts({ unread: true })).toEqual([{ field: 'unread', value: '' }]);
    expect(researchFilterParts({ unread: false })).toEqual([]);
  });

  it('drops keys it cannot show and values that are not text', () => {
    expect(researchFilterParts({ order: 'oldest', from: 42, subject: '  ', to: 'a@example.com' })).toEqual([
      { field: 'to', value: 'a@example.com' },
    ]);
  });

  it('keeps a date it cannot parse as written', () => {
    expect(researchFilterParts({ since: 'last week' })).toEqual([{ field: 'since', value: 'last week' }]);
  });
});
