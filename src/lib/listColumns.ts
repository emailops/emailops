// The column header of the full-width (single-line) mail list: the order the
// list is sorted in, and which optional columns it shows.
//
// The sort is applied by the backend query (`get_emails`), so it holds across
// every page, not only the rows already loaded.

import type { MailboxView } from '@/lib/api';

export type ListSortKey = 'date' | 'sender' | 'subject' | 'size';

export interface ListSort {
  key: ListSortKey;
  dir: 'asc' | 'desc';
}

/** Newest first: the order every list had before it could be sorted. */
export const DEFAULT_LIST_SORT: ListSort = { key: 'date', dir: 'desc' };

/** The order after a click on a header: the sorted column flips; another
 *  column starts ascending (A to Z, smallest first), Date newest first. */
export function nextListSort(current: ListSort, key: ListSortKey): ListSort {
  if (current.key === key) return { key, dir: current.dir === 'asc' ? 'desc' : 'asc' };
  return key === 'date' ? DEFAULT_LIST_SORT : { key, dir: 'asc' };
}

/** Whether the list on screen can be sorted by its headers. A search or a
 *  filter keeps its own order, and the Snoozed view lists by wake time. */
export function listSortable(view: MailboxView, hasQuery: boolean): boolean {
  return !hasQuery && view !== 'snoozed';
}

/** The optional columns, saved in the `list_columns` preference as JSON. */
export interface ListColumns {
  size: boolean;
  attachment: boolean;
}

export const LIST_COLUMNS_PREF = 'list_columns';

export const DEFAULT_LIST_COLUMNS: ListColumns = { size: false, attachment: false };

/** Read the saved preference; `null` (the default applies) when it is not a
 *  `{size, attachment}` object of booleans. A missing column is off. */
export function parseListColumns(raw: string): ListColumns | null {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof value !== 'object' || value === null) return null;
  const { size = false, attachment = false } = value as Record<string, unknown>;
  if (typeof size !== 'boolean' || typeof attachment !== 'boolean') return null;
  return { size, attachment };
}

/** Widths shared by the column header and the single-line rows, so their
 *  cells line up. Complete literals: Tailwind only generates classes it finds
 *  written out. The party and date widths are the ones the rows had. */
export const LIST_COLUMN_WIDTH = {
  party: 'w-24 @2xl:w-28 @4xl:w-44',
  size: 'w-16',
  attachment: 'w-4',
  date: 'w-20 @2xl:w-24',
} as const;
