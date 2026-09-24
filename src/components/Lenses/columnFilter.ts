// Excel-style column filter: which rows a set of ticked values keeps, and how
// a raw column value reads in the value list. Values arrive as the text the
// backend compares on (`CAST(... AS TEXT)` of the stored JSON), so booleans
// come as "1"/"0" and amounts as their JSON object.

import type { LensColumnFilter, LensColumnType } from '@/types';

/** The filter a set of ticked values means. Everything ticked is no filter. */
export function filterFromSelection(
  key: string,
  allValues: (string | null)[],
  selected: Set<string | null>,
): LensColumnFilter | null {
  if (allValues.every((v) => selected.has(v))) return null;
  return {
    key,
    values: allValues.filter((v): v is string => v !== null && selected.has(v)),
    includeEmpty: selected.has(null),
  };
}

/** How one value reads in the filter list. */
export function columnValueLabel(
  value: string | null,
  type: LensColumnType,
  labels: { empty: string; yes: string; no: string },
): string {
  if (value === null) return labels.empty;
  if (type === 'boolean') return value === '1' ? labels.yes : labels.no;
  if (type === 'currency') {
    try {
      const parsed = JSON.parse(value) as { amount?: unknown; currency?: unknown };
      if (typeof parsed.amount === 'number') {
        return [parsed.amount, typeof parsed.currency === 'string' ? parsed.currency : ''].join(' ').trim();
      }
    } catch {
      // A bare number or text: shown as is below.
    }
  }
  return value;
}
