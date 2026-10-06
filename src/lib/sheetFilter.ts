/**
 * Column filters of an EO Docs sheet, as in Excel: the first row is the
 * header and always shows; below it, a column with a filter shows only the
 * rows whose value is ticked. Filters belong to the person looking — they
 * live in the editor, not in the shared document.
 */

/** Column index → the values to show. A column absent here is not filtered. */
export type ColumnFilters = Record<number, Set<string>>;

/** Pure: the distinct values of a column below the header, blank last. */
export function columnValues(values: string[][], col: number): string[] {
  const seen = new Set<string>();
  for (const row of values.slice(1)) seen.add(row[col] ?? '');
  const list = [...seen].filter((v) => v !== '').sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
  return seen.has('') ? [...list, ''] : list;
}

/** Pure: the indexes of the rows to show. */
export function visibleRows(values: string[][], filters: ColumnFilters): number[] {
  const active = Object.entries(filters).map(([col, allowed]) => [Number(col), allowed] as const);
  return values
    .map((_, r) => r)
    .filter((r) => r === 0 || active.every(([col, allowed]) => allowed.has(values[r][col] ?? '')));
}
