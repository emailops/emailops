import { type KeyboardEvent as ReactKeyboardEvent, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type * as Y from 'yjs';
import { isTablePaste, parseClipboardTable } from '@/lib/clipboardTable';
import { type ColumnFilters, columnValues, visibleRows } from '@/lib/sheetFilter';
import { displayValue } from '@/lib/sheetFormula';
import {
  columnLabel,
  deleteColumn,
  deleteRow,
  ensureGrid,
  type Grid,
  insertColumn,
  insertRow,
  MAX_COL_WIDTH,
  MIN_COL_WIDTH,
  newId,
  pasteBlock,
  readGrid,
  setCell,
  setColumnWidth,
  sheetUndoManager,
} from '@/lib/sheetModel';

interface SheetEditorProps {
  doc: Y.Doc;
  editable: boolean;
}

/** Move the focus to the cell input at (`row`, `col`), if there is one. */
function focusCell(row: number, col: number) {
  document.querySelector<HTMLInputElement>(`[data-cell="${row}:${col}"]`)?.focus();
}

interface ColumnFilterMenuProps {
  column: string;
  values: string[];
  /** The values shown; `undefined` when the column is not filtered. */
  selected: Set<string> | undefined;
  onChange: (selected: Set<string> | undefined) => void;
  onClose: () => void;
}

/** The checklist under a column header, as Excel's AutoFilter. */
function ColumnFilterMenu({ column, values, selected, onChange, onClose }: ColumnFilterMenuProps) {
  const { t } = useTranslation(['documents']);
  const ref = useRef<HTMLDivElement>(null);
  const shown = selected ?? new Set(values);

  useEffect(() => {
    const close = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('mousedown', close);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', close);
      window.removeEventListener('keydown', onKey);
    };
  }, [onClose]);

  const toggle = (value: string) => {
    const next = new Set(shown);
    if (next.has(value)) next.delete(value);
    else next.add(value);
    onChange(next.size === values.length ? undefined : next);
  };

  return (
    <div
      ref={ref}
      role="dialog"
      aria-label={t('documents:sheet.filterColumn', { col: column })}
      data-testid="sheet-filter-menu"
      className="absolute left-0 top-full z-20 mt-1 w-56 rounded border border-gray-600 bg-gray-800 p-2 text-left text-xs font-normal text-gray-200 shadow-lg"
    >
      <div className="mb-1 flex gap-2">
        <button
          type="button"
          onClick={() => onChange(undefined)}
          className="rounded px-1.5 py-0.5 text-gray-300 hover:bg-gray-700"
        >
          {t('documents:sheet.selectAll')}
        </button>
        <button
          type="button"
          data-testid="sheet-filter-clear-all"
          onClick={() => onChange(new Set())}
          className="rounded px-1.5 py-0.5 text-gray-300 hover:bg-gray-700"
        >
          {t('documents:sheet.selectNone')}
        </button>
      </div>
      <ul className="max-h-56 overflow-y-auto">
        {values.map((value) => (
          <li key={value}>
            <label className="flex items-center gap-2 rounded px-1 py-0.5 hover:bg-gray-700">
              <input
                type="checkbox"
                data-testid={`sheet-filter-value-${value}`}
                checked={shown.has(value)}
                onChange={() => toggle(value)}
              />
              <span className="truncate">{value === '' ? t('documents:sheet.blank') : value}</span>
            </label>
          </li>
        ))}
      </ul>
    </div>
  );
}

interface Resize {
  colId: string;
  startX: number;
  startWidth: number;
  width: number;
}

/**
 * A grid over the shared `Y.Doc` (see `sheetModel`): each cell is an input
 * that writes straight into the document, and the grid re-reads the document
 * after every change, local or merged from someone else.
 *
 * - A value starting with "=" is a formula (`sheetFormula`): the cell shows
 *   its result, and the formula while the cell has the focus.
 * - Column widths are dragged from the right edge of a header and shared.
 * - Column filters (first row = header) are this person's view only. A row
 *   added while filtering stays in view until the filters change.
 * - Undo and redo (toolbar, Cmd/Ctrl+Z, Cmd/Ctrl+Shift+Z, Ctrl+Y) cover this
 *   person's own changes only.
 */
export function SheetEditor({ doc, editable }: SheetEditorProps) {
  const { t, i18n } = useTranslation(['documents']);
  const [grid, setGrid] = useState<Grid>(() => readGrid(doc));
  const [focused, setFocused] = useState<string | null>(null);
  const [filters, setFilters] = useState<Record<string, Set<string>>>({});
  const [filterOpen, setFilterOpen] = useState<string | null>(null);
  const [resize, setResize] = useState<Resize | null>(null);
  const [added, setAdded] = useState<Set<string>>(new Set());
  const [undo, setUndo] = useState<Y.UndoManager | null>(null);
  const [history, setHistory] = useState({ canUndo: false, canRedo: false });
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const manager = sheetUndoManager(doc);
    const track = () => setHistory({ canUndo: manager.canUndo(), canRedo: manager.canRedo() });
    manager.on('stack-item-added', track);
    manager.on('stack-item-popped', track);
    manager.on('stack-cleared', track);
    setUndo(manager);
    track();
    return () => {
      manager.destroy();
      setUndo(null);
    };
  }, [doc]);

  // The webview's own Edit menu sends undo as an input event, not a key press.
  useEffect(() => {
    const root = rootRef.current;
    if (!root || !undo || !editable) return;
    const onBeforeInput = (e: InputEvent) => {
      if (e.inputType !== 'historyUndo' && e.inputType !== 'historyRedo') return;
      e.preventDefault();
      if (e.inputType === 'historyUndo') undo.undo();
      else undo.redo();
    };
    root.addEventListener('beforeinput', onBeforeInput);
    return () => root.removeEventListener('beforeinput', onBeforeInput);
  }, [undo, editable]);

  useEffect(() => {
    // A sheet nobody has laid out yet gets its first rows and columns here.
    if (editable) ensureGrid(doc);
    const refresh = () => setGrid(readGrid(doc));
    refresh();
    doc.on('update', refresh);
    return () => doc.off('update', refresh);
  }, [doc, editable]);

  // Dragging a column edge: the width follows the pointer here and is written
  // to the document once, on release.
  useEffect(() => {
    if (!resize) return;
    const move = (e: PointerEvent) => {
      const width = Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, resize.startWidth + e.clientX - resize.startX));
      setResize((r) => (r ? { ...r, width } : r));
    };
    const up = () => {
      setColumnWidth(doc, resize.colId, resize.width);
      setResize(null);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
  }, [resize, doc]);

  const lastRow = grid.rowIds.length;
  const lastCol = grid.colIds.length;
  const byIndex: ColumnFilters = {};
  grid.colIds.forEach((colId, c) => {
    if (filters[colId]) byIndex[c] = filters[colId];
  });
  const shown = visibleRows(grid.values, byIndex);
  const rows = grid.rowIds.flatMap((rowId, r) => (shown.includes(r) || added.has(rowId) ? [r] : []));
  const widthOf = (colId: string, c: number) => (resize?.colId === colId ? resize.width : grid.widths[c]);
  const setFilter = (colId: string, selected: Set<string> | undefined) => {
    setAdded(new Set());
    setFilters(({ [colId]: _, ...rest }) => (selected ? { ...rest, [colId]: selected } : rest));
  };
  const addRow = (index: number) =>
    insertRow(doc, index, () => {
      const id = newId();
      setAdded((ids) => new Set(ids).add(id));
      return id;
    });
  const onKeyDown = (e: ReactKeyboardEvent) => {
    if (!undo || !editable || !(e.metaKey || e.ctrlKey)) return;
    const key = e.key.toLowerCase();
    if (key === 'z' && !e.shiftKey) undo.undo();
    else if ((key === 'z' && e.shiftKey) || key === 'y') undo.redo();
    else return;
    e.preventDefault();
  };

  return (
    <div ref={rootRef} onKeyDown={onKeyDown} className="flex-1 min-h-0 overflow-auto p-4" data-testid="shared-sheet">
      <div className="mb-2 flex items-center gap-3 min-h-6">
        {editable && (
          <div className="flex gap-1">
            <button
              type="button"
              data-testid="sheet-undo"
              title={t('documents:toolbar.undo')}
              aria-label={t('documents:toolbar.undo')}
              disabled={!history.canUndo}
              onClick={() => undo?.undo()}
              className="px-2 py-0.5 rounded text-sm text-gray-300 hover:bg-gray-700 disabled:opacity-40 disabled:hover:bg-transparent"
            >
              ↶
            </button>
            <button
              type="button"
              data-testid="sheet-redo"
              title={t('documents:toolbar.redo')}
              aria-label={t('documents:toolbar.redo')}
              disabled={!history.canRedo}
              onClick={() => undo?.redo()}
              className="px-2 py-0.5 rounded text-sm text-gray-300 hover:bg-gray-700 disabled:opacity-40 disabled:hover:bg-transparent"
            >
              ↷
            </button>
          </div>
        )}
        {rows.length < grid.rowIds.length && (
          <p className="text-xs text-amber-300" data-testid="sheet-filtered">
            {t('documents:sheet.filteredRows', { shown: rows.length - 1, total: grid.rowIds.length - 1 })}
          </p>
        )}
      </div>
      <table className="border-collapse text-sm text-gray-200" style={{ tableLayout: 'fixed' }}>
        <colgroup>
          <col style={{ width: 48 }} />
          {grid.colIds.map((colId, c) => (
            <col key={colId} style={{ width: widthOf(colId, c) }} />
          ))}
          {editable && <col style={{ width: 32 }} />}
        </colgroup>
        <thead>
          <tr>
            <th />
            {grid.colIds.map((colId, c) => (
              <th
                key={colId}
                className="group relative px-2 py-1 border border-gray-700 bg-gray-800 font-medium text-gray-400"
              >
                <span>{columnLabel(c)}</span>
                <button
                  type="button"
                  data-testid={`sheet-filter-${c}`}
                  title={t('documents:sheet.filterColumn', { col: columnLabel(c) })}
                  aria-label={t('documents:sheet.filterColumn', { col: columnLabel(c) })}
                  aria-pressed={!!filters[colId]}
                  onClick={() => setFilterOpen((open) => (open === colId ? null : colId))}
                  className={`ml-1 ${filters[colId] ? 'text-primary-400' : 'invisible group-hover:visible text-gray-500 hover:text-white'}`}
                >
                  <svg className="inline w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={2}
                      d="M3 5h18l-7 8v6l-4 2v-8L3 5z"
                    />
                  </svg>
                </button>
                {editable && (
                  <button
                    type="button"
                    title={t('documents:sheet.deleteColumn')}
                    aria-label={t('documents:sheet.deleteColumn')}
                    onClick={() => deleteColumn(doc, c)}
                    className="ml-1 invisible group-hover:visible text-gray-500 hover:text-red-300"
                  >
                    ×
                  </button>
                )}
                {editable && (
                  <span
                    data-testid={`sheet-resize-${c}`}
                    title={t('documents:sheet.resizeColumn', { col: columnLabel(c) })}
                    onPointerDown={(e) => {
                      e.preventDefault();
                      const width = widthOf(colId, c);
                      setResize({ colId, startX: e.clientX, startWidth: width, width });
                    }}
                    className="absolute right-0 top-0 h-full w-1.5 cursor-col-resize hover:bg-primary-500"
                  />
                )}
                {filterOpen === colId && (
                  <ColumnFilterMenu
                    column={columnLabel(c)}
                    values={columnValues(grid.values, c)}
                    selected={filters[colId]}
                    onChange={(selected) => setFilter(colId, selected)}
                    onClose={() => setFilterOpen(null)}
                  />
                )}
              </th>
            ))}
            {editable && (
              <th>
                <button
                  type="button"
                  data-testid="sheet-add-column"
                  title={t('documents:sheet.addColumn')}
                  aria-label={t('documents:sheet.addColumn')}
                  onClick={() => insertColumn(doc, lastCol)}
                  className="px-2 text-gray-400 hover:text-white"
                >
                  +
                </button>
              </th>
            )}
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => {
            const rowId = grid.rowIds[r];
            return (
              <tr key={rowId} className="group">
                <th className="px-2 border border-gray-700 bg-gray-800 font-medium text-gray-400 text-right whitespace-nowrap">
                  {editable && (
                    <button
                      type="button"
                      data-testid={`sheet-insert-row-${r}`}
                      title={t('documents:sheet.insertRowAbove')}
                      aria-label={t('documents:sheet.insertRowAbove')}
                      onClick={() => addRow(r)}
                      className="mr-1 invisible group-hover:visible text-gray-500 hover:text-white"
                    >
                      +
                    </button>
                  )}
                  {editable && (
                    <button
                      type="button"
                      title={t('documents:sheet.deleteRow')}
                      aria-label={t('documents:sheet.deleteRow')}
                      onClick={() => deleteRow(doc, r)}
                      className="mr-1 invisible group-hover:visible text-gray-500 hover:text-red-300"
                    >
                      ×
                    </button>
                  )}
                  {r + 1}
                </th>
                {grid.colIds.map((colId, c) => {
                  const ref = `${r}:${c}`;
                  const raw = grid.values[r]?.[c] ?? '';
                  const isFormula = raw.startsWith('=');
                  return (
                    <td key={colId} className="border border-gray-700 p-0">
                      <input
                        data-cell={ref}
                        aria-label={t('documents:sheet.cell', { ref: `${columnLabel(c)}${r + 1}` })}
                        value={focused === ref || !isFormula ? raw : displayValue(grid.values, r, c, i18n.language)}
                        readOnly={!editable}
                        title={isFormula ? raw : undefined}
                        onFocus={() => setFocused(ref)}
                        onBlur={() => setFocused((f) => (f === ref ? null : f))}
                        onChange={(e) => setCell(doc, rowId, colId, e.target.value)}
                        onPaste={(e) => {
                          // A block copied from a spreadsheet fills the cells from
                          // here on; plain text goes into this cell as usual.
                          const text = e.clipboardData.getData('text/plain');
                          if (!editable || !isTablePaste(text)) return;
                          e.preventDefault();
                          pasteBlock(doc, r, c, parseClipboardTable(text));
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const at = rows.indexOf(r);
                            const next = rows[e.shiftKey ? at - 1 : at + 1];
                            if (next !== undefined) focusCell(next, c);
                          }
                        }}
                        className={`w-full px-2 py-1 bg-transparent focus:bg-gray-800 focus:outline-none focus:ring-1 focus:ring-primary-500 ${
                          isFormula && focused !== ref ? 'text-right text-primary-200' : ''
                        }`}
                      />
                    </td>
                  );
                })}
              </tr>
            );
          })}
        </tbody>
      </table>
      {editable && (
        <button
          type="button"
          data-testid="sheet-add-row"
          onClick={() => addRow(lastRow)}
          className="mt-2 px-2 py-1 text-xs rounded text-gray-400 hover:text-white hover:bg-gray-700"
        >
          + {t('documents:sheet.addRow')}
        </button>
      )}
    </div>
  );
}
