import * as Y from 'yjs';
import { columnName, displayValue, shiftFormula } from '@/lib/sheetFormula';

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
  /** Column widths in pixels, shared with everyone editing the sheet. */
  widths: number[];
}

const ROWS = 'rows';
const COLS = 'cols';
const CELLS = 'cells';
const WIDTHS = 'colWidths';

export const DEFAULT_COL_WIDTH = 128;
export const MIN_COL_WIDTH = 40;
export const MAX_COL_WIDTH = 800;

export const DEFAULT_ROWS = 20;
export const DEFAULT_COLS = 8;

function parts(doc: Y.Doc) {
  return {
    rows: doc.getArray<string>(ROWS),
    cols: doc.getArray<string>(COLS),
    cells: doc.getMap<string>(CELLS),
    widths: doc.getMap<number>(WIDTHS),
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
    widths: colIds.map((c) => p.widths.get(c) ?? DEFAULT_COL_WIDTH),
  };
}

/** Undo and redo for this person's own changes to the sheet: changes merged
 *  in from other people carry an origin, so they are never undone here. */
export function sheetUndoManager(doc: Y.Doc): Y.UndoManager {
  const p = parts(doc);
  return new Y.UndoManager([p.rows, p.cols, p.cells, p.widths]);
}

/** Set one cell; an empty value removes it. */
export function setCell(doc: Y.Doc, rowId: string, colId: string, value: string): void {
  const { cells } = parts(doc);
  if (value === '') cells.delete(key(rowId, colId));
  else if (cells.get(key(rowId, colId)) !== value) cells.set(key(rowId, colId), value);
}

/** Set a column's width, kept between a usable minimum and maximum. */
export function setColumnWidth(doc: Y.Doc, colId: string, px: number): void {
  const width = Math.round(Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, px)));
  const { widths } = parts(doc);
  if (widths.get(colId) !== width) widths.set(colId, width);
}

/** Rewrite every formula for a row or column inserted or deleted at `index`,
 *  so its references keep pointing at the same cells. */
function shiftFormulas(doc: Y.Doc, axis: 'row' | 'col', index: number, delta: 1 | -1) {
  const { cells } = parts(doc);
  for (const [k, value] of cells.entries()) {
    const shifted = shiftFormula(value, axis, index, delta);
    if (shifted !== value) cells.set(k, shifted);
  }
}

export function insertRow(doc: Y.Doc, index: number, makeId = newId): void {
  doc.transact(() => {
    parts(doc).rows.insert(index, [makeId()]);
    shiftFormulas(doc, 'row', index, 1);
  });
}

export function insertColumn(doc: Y.Doc, index: number, makeId = newId): void {
  doc.transact(() => {
    parts(doc).cols.insert(index, [makeId()]);
    shiftFormulas(doc, 'col', index, 1);
  });
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
  doc.transact(() => {
    deleteLine(doc, parts(doc).rows, index, (row, col) => key(row, col));
    shiftFormulas(doc, 'row', index, -1);
  });
}

export function deleteColumn(doc: Y.Doc, index: number): void {
  const p = parts(doc);
  const colId = p.cols.get(index);
  doc.transact(() => {
    deleteLine(doc, p.cols, index, (col, row) => key(row, col));
    if (colId !== undefined) p.widths.delete(colId);
    shiftFormulas(doc, 'col', index, -1);
  });
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
      line.forEach((value, c) => {
        setCell(doc, rowIds[row + r], colIds[col + c], value);
      });
    });
  });
}

/** Spreadsheet column name: 0 → A, 25 → Z, 26 → AA. */
export const columnLabel = columnName;

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

/** The sheet as an HTML table: the readable copy an invitation carries. Empty
 *  rows and columns after the last value are left out. */
export function gridToHtml(grid: Grid, locale = 'en'): string {
  const rows = grid.values.reduce((n, row, r) => (row.some((v) => v) ? r + 1 : n), 0);
  const cols = grid.values.reduce((n, row) => row.reduce((m, v, c) => (v ? Math.max(m, c + 1) : m), n), 0);
  const head = Array.from({ length: cols }, (_, i) => `<th>${columnLabel(i)}</th>`).join('');
  const body = grid.values
    .slice(0, rows)
    .map(
      (row, r) =>
        `<tr>${row
          .slice(0, cols)
          .map((_, c) => `<td>${escapeHtml(displayValue(grid.values, r, c, locale))}</td>`)
          .join('')}</tr>`,
    )
    .join('');
  return `<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`;
}
