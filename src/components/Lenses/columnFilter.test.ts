import { describe, expect, it } from 'vitest';

import { columnValueLabel, filterFromSelection } from './columnFilter';

describe('filterFromSelection', () => {
  const all = [null, 'Acme', 'Globex'];

  it('means no filter when every value is ticked', () => {
    expect(filterFromSelection('vendor', all, new Set(all))).toBeNull();
  });

  it('keeps the ticked values and whether empty cells stay', () => {
    expect(filterFromSelection('vendor', all, new Set(['Acme', null]))).toEqual({
      key: 'vendor',
      values: ['Acme'],
      includeEmpty: true,
    });
  });

  it('keeps no rows when nothing is ticked, like Excel', () => {
    expect(filterFromSelection('vendor', all, new Set())).toEqual({ key: 'vendor', values: [], includeEmpty: false });
  });
});

describe('columnValueLabel', () => {
  const t = { empty: '(Empty)', yes: 'Yes', no: 'No' };

  it('names empty cells', () => {
    expect(columnValueLabel(null, 'string', t)).toBe('(Empty)');
  });

  it('reads booleans as yes/no', () => {
    expect(columnValueLabel('1', 'boolean', t)).toBe('Yes');
    expect(columnValueLabel('0', 'boolean', t)).toBe('No');
  });

  it('shows a stored amount with its currency', () => {
    expect(columnValueLabel('{"amount":12.5,"currency":"EUR"}', 'currency', t)).toBe('12.5 EUR');
  });

  it('passes other values through', () => {
    expect(columnValueLabel('quote_request', 'enum', t)).toBe('quote_request');
  });
});
