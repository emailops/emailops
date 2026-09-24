import { describe, expect, it } from 'vitest';

import { DATE_SORT_KEY, nextSort } from './lensSort';

describe('nextSort', () => {
  it('cycles a column through its first direction, the other one, and back to the default', () => {
    const first = nextSort(null, 'amount', 'desc');
    expect(first).toEqual({ columnKey: 'amount', direction: 'desc' });
    const second = nextSort(first, 'amount', 'desc');
    expect(second).toEqual({ columnKey: 'amount', direction: 'asc' });
    expect(nextSort(second, 'amount', 'desc')).toBeNull();
  });

  it('starts the date column oldest-first, since the default order is already newest-first', () => {
    expect(nextSort(null, DATE_SORT_KEY, 'asc')).toEqual({ columnKey: DATE_SORT_KEY, direction: 'asc' });
    expect(nextSort({ columnKey: DATE_SORT_KEY, direction: 'asc' }, DATE_SORT_KEY, 'asc')).toEqual({
      columnKey: DATE_SORT_KEY,
      direction: 'desc',
    });
  });

  it('switching to another column starts that column fresh', () => {
    expect(nextSort({ columnKey: 'amount', direction: 'asc' }, DATE_SORT_KEY, 'asc')).toEqual({
      columnKey: DATE_SORT_KEY,
      direction: 'asc',
    });
  });
});
