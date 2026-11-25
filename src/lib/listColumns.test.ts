import { describe, expect, it } from 'vitest';
import { DEFAULT_LIST_COLUMNS, DEFAULT_LIST_SORT, listSortable, nextListSort, parseListColumns } from './listColumns';

describe('DEFAULT_LIST_SORT', () => {
  it('lists the newest first', () => {
    expect(DEFAULT_LIST_SORT).toEqual({ key: 'date', dir: 'desc' });
  });
});

describe('nextListSort', () => {
  it('flips the direction of the column already sorted', () => {
    expect(nextListSort({ key: 'size', dir: 'asc' }, 'size')).toEqual({ key: 'size', dir: 'desc' });
    expect(nextListSort({ key: 'size', dir: 'desc' }, 'size')).toEqual({ key: 'size', dir: 'asc' });
  });

  it('turns newest first into oldest first on a click on Date', () => {
    expect(nextListSort(DEFAULT_LIST_SORT, 'date')).toEqual({ key: 'date', dir: 'asc' });
  });

  it.each(['sender', 'subject', 'size'] as const)('starts %s ascending', (key) => {
    expect(nextListSort(DEFAULT_LIST_SORT, key)).toEqual({ key, dir: 'asc' });
  });

  it('starts Date newest first when another column was sorted', () => {
    expect(nextListSort({ key: 'subject', dir: 'asc' }, 'date')).toEqual(DEFAULT_LIST_SORT);
  });
});

describe('listSortable', () => {
  it.each(['inbox', 'sent', 'archive', 'spam', 'deleted', 'starred', 'folder:Work'] as const)(
    'sorts the %s list',
    (view) => {
      expect(listSortable(view, false)).toBe(true);
    },
  );

  it('keeps the Snoozed view in wake order', () => {
    expect(listSortable('snoozed', false)).toBe(false);
  });

  it('keeps a search or filter in its own order', () => {
    expect(listSortable('inbox', true)).toBe(false);
  });
});

describe('parseListColumns', () => {
  it('reads a saved choice', () => {
    expect(parseListColumns('{"size":true,"attachment":false}')).toEqual({ size: true, attachment: false });
  });

  it('turns a column it does not find off', () => {
    expect(parseListColumns('{"attachment":true}')).toEqual({ size: false, attachment: true });
  });

  it.each([
    ['malformed JSON', '{size'],
    ['a non-object', '"size"'],
    ['null', 'null'],
    ['a non-boolean flag', '{"size":"yes"}'],
  ])('rejects %s', (_label, raw) => {
    expect(parseListColumns(raw)).toBeNull();
  });

  it('shows neither optional column by default', () => {
    expect(DEFAULT_LIST_COLUMNS).toEqual({ size: false, attachment: false });
  });
});
