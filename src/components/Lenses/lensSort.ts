import type { LensSortSpec } from '@/types';

/** Sort key the backend reads as the email's date (`email_timestamp`). */
export const DATE_SORT_KEY = 'emailTimestamp';

/**
 * Next sort after a header click: a new column starts in `first`, a second
 * click flips it, a third returns to the default order (newest email first).
 */
export function nextSort(current: LensSortSpec | null, key: string, first: 'asc' | 'desc'): LensSortSpec | null {
  if (!current || current.columnKey !== key) return { columnKey: key, direction: first };
  if (current.direction === first) return { columnKey: key, direction: first === 'asc' ? 'desc' : 'asc' };
  return null;
}
