import type * as Y from 'yjs';

/**
 * A shared sheet inside a Yjs document. Rows and columns are ordered lists of
 * stable ids (`rows`, `cols`), and each non-empty cell is stored under
 * `"<rowId>:<colId>"` in the `cells` map. Keying cells by id rather than by
 * position is what lets two people insert rows at the same time without one
 * person's values sliding into the other's rows.
 *
 * Every function here is pure over the `Y.Doc` it is given; the backend only
 * stores and forwards the bytes.
 */

export interface Grid {
  rowIds: string[];
  colIds: string[];
  /** `values[row][col]`, `''` for an empty cell. */
  values: string[][];
}

const ROWS = 'rows';
const COLS = 'cols';
const CELLS = 'cells';

export const DEFAULT_ROWS = 20;
export const DEFAULT_COLS = 8;

function parts(doc: Y.Doc) {
  return {
    rows: doc.getArray<string>(ROWS),
    cols: doc.getArray<string>(COLS),
    cells: doc.getMap<string>(CELLS),
  };
}

export function newId(): string {
  return crypto.randomUUID();
}

function key(rowId: string, colId: string): string {
  return `${rowId}:${colId}`;
}

/** Lay out an empty sheet; a sheet that already has rows or columns is left as it is. */
export function ensureGrid(doc: Y.Doc, rows = DEFAULT_ROWS, cols = DEFAULT_COLS, makeId = newId): void {
  const p = parts(doc);
  if (p.rows.length > 0 || p.cols.length > 0) return;
  doc.transact(() => {
    p.rows.push(Array.from({ length: rows }, makeId));
    p.cols.push(Array.from({ length: cols }, makeId));
  });
}

export function readGrid(doc: Y.Doc): Grid {
  const p = parts(doc);
  const rowIds = p.rows.toArray();
  const colIds = p.cols.toArray();
  return {
    rowIds,
    colIds,
    values: rowIds.map((r) => colIds.map((c) => p.cells.get(key(r, c)) ?? '')),
  };
}

/** Set one cell; an empty value removes it. */
export function setCell(doc: Y.Doc, rowId: string, colId: string, value: string): void {
  const { cells } = parts(doc);
  if (value === '') cells.delete(key(rowId, colId));
  else if (cells.get(key(rowId, colId)) !== value) cells.set(key(rowId, colId), value);
}

export function insertRow(doc: Y.Doc, index: number, makeId = newId): void {
  parts(doc).rows.insert(index, [makeId()]);
}

export function insertColumn(doc: Y.Doc, index: number, makeId = newId): void {
  parts(doc).cols.insert(index, [makeId()]);
}

function deleteLine(doc: Y.Doc, list: Y.Array<string>, index: number, keyOf: (id: string, other: string) => string) {
  const id = list.get(index);
  if (id === undefined) return;
  const p = parts(doc);
  const others = list === p.rows ? p.cols.toArray() : p.rows.toArray();
  doc.transact(() => {
    list.delete(index, 1);
    for (const other of others) p.cells.delete(keyOf(id, other));
  });
}

export function deleteRow(doc: Y.Doc, index: number): void {
  deleteLine(doc, parts(doc).rows, index, (row, col) => key(row, col));
}

export function deleteColumn(doc: Y.Doc, index: number): void {
  deleteLine(doc, parts(doc).cols, index, (col, row) => key(row, col));
}

/**
 * Paste a block of values with its top-left corner at (`row`, `col`), as one
 * change: rows and columns are added when the block runs past the grid, and a
 * value pasted empty clears its cell.
 */
export function pasteBlock(doc: Y.Doc, row: number, col: number, values: string[][], makeId = newId): void {
  const p = parts(doc);
  const width = values.reduce((w, r) => Math.max(w, r.length), 0);
  doc.transact(() => {
    while (p.rows.length < row + values.length) p.rows.push([makeId()]);
    while (p.cols.length < col + width) p.cols.push([makeId()]);
    const rowIds = p.rows.toArray();
    const colIds = p.cols.toArray();
    values.forEach((line, r) => {
      line.forEach((value, c) => setCell(doc, rowIds[row + r], colIds[col + c], value));
    });
  });
}

/** Spreadsheet column name: 0 → A, 25 → Z, 26 → AA. */
export function columnLabel(index: number): string {
  let n = index + 1;
  let label = '';
  while (n > 0) {
    const rem = (n - 1) % 26;
    label = String.fromCharCode(65 + rem) + label;
    n = Math.floor((n - 1) / 26);
  }
  return label;
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** The sheet as an HTML table: the readable copy an invitation carries. Empty
 *  rows and columns after the last value are left out. */
export function gridToHtml(grid: Grid): string {
  const rows = grid.values.reduce((n, row, r) => (row.some((v) => v) ? r + 1 : n), 0);
  const cols = grid.values.reduce((n, row) => row.reduce((m, v, c) => (v ? Math.max(m, c + 1) : m), n), 0);
  const head = Array.from({ length: cols }, (_, i) => `<th>${columnLabel(i)}</th>`).join('');
  const body = grid.values
    .slice(0, rows)
    .map(
      (row) =>
        `<tr>${row
          .slice(0, cols)
          .map((v) => `<td>${escapeHtml(v)}</td>`)
          .join('')}</tr>`,
    )
    .join('');
  return `<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`;
}
