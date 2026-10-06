import { describe, expect, it } from 'vitest';
import { type ColumnFilters, columnValues, visibleRows } from './sheetFilter';

const VALUES = [
  ['Item', 'Priority'],
  ['Desk', 'High'],
  ['Lamp', 'Low'],
  ['Chair', 'High'],
  ['Shelf', ''],
];

describe('columnValues', () => {
  it('lists the distinct values below the header, blanks last', () => {
    expect(columnValues(VALUES, 1)).toEqual(['High', 'Low', '']);
  });
});

describe('visibleRows', () => {
  it('shows every row with no filter', () => {
    expect(visibleRows(VALUES, {})).toEqual([0, 1, 2, 3, 4]);
  });

  it('keeps the header and the rows whose value is ticked', () => {
    const filters: ColumnFilters = { 1: new Set(['High']) };
    expect(visibleRows(VALUES, filters)).toEqual([0, 1, 3]);
  });

  it('combines filters on several columns', () => {
    const filters: ColumnFilters = { 0: new Set(['Desk', 'Lamp']), 1: new Set(['High', '']) };
    expect(visibleRows(VALUES, filters)).toEqual([0, 1]);
  });
});
