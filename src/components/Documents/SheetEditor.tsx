import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type * as Y from 'yjs';
import {
  columnLabel,
  deleteColumn,
  deleteRow,
  ensureGrid,
  type Grid,
  insertColumn,
  insertRow,
  readGrid,
  setCell,
} from '@/lib/sheetModel';

interface SheetEditorProps {
  doc: Y.Doc;
  editable: boolean;
}

/** Move the focus to the cell input at (`row`, `col`), if there is one. */
function focusCell(row: number, col: number) {
  document.querySelector<HTMLInputElement>(`[data-cell="${row}:${col}"]`)?.focus();
}

/**
 * A plain grid over the shared `Y.Doc` (see `sheetModel`): each cell is an
 * input that writes straight into the document, and the grid re-reads the
 * document after every change, local or merged from someone else. Values only,
 * no formulas.
 */
export function SheetEditor({ doc, editable }: SheetEditorProps) {
  const { t } = useTranslation(['documents']);
  const [grid, setGrid] = useState<Grid>(() => readGrid(doc));

  useEffect(() => {
    // A sheet nobody has laid out yet gets its first rows and columns here.
    if (editable) ensureGrid(doc);
    const refresh = () => setGrid(readGrid(doc));
    refresh();
    doc.on('update', refresh);
    return () => doc.off('update', refresh);
  }, [doc, editable]);

  const lastRow = grid.rowIds.length;
  const lastCol = grid.colIds.length;

  return (
    <div className="flex-1 min-h-0 overflow-auto p-4" data-testid="shared-sheet">
      <table className="border-collapse text-sm text-gray-200">
        <thead>
          <tr>
            <th className="w-10" />
            {grid.colIds.map((colId, c) => (
              <th key={colId} className="group px-2 py-1 border border-gray-700 bg-gray-800 font-medium text-gray-400">
                <span>{columnLabel(c)}</span>
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
          {grid.rowIds.map((rowId, r) => (
            <tr key={rowId} className="group">
              <th className="px-2 border border-gray-700 bg-gray-800 font-medium text-gray-400 text-right whitespace-nowrap">
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
              {grid.colIds.map((colId, c) => (
                <td key={colId} className="border border-gray-700 p-0">
                  <input
                    data-cell={`${r}:${c}`}
                    aria-label={t('documents:sheet.cell', { ref: `${columnLabel(c)}${r + 1}` })}
                    value={grid.values[r]?.[c] ?? ''}
                    readOnly={!editable}
                    onChange={(e) => setCell(doc, rowId, colId, e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') {
                        e.preventDefault();
                        focusCell(e.shiftKey ? r - 1 : r + 1, c);
                      }
                    }}
                    className="w-32 px-2 py-1 bg-transparent focus:bg-gray-800 focus:outline-none focus:ring-1 focus:ring-primary-500"
                  />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {editable && (
        <button
          type="button"
          data-testid="sheet-add-row"
          onClick={() => insertRow(doc, lastRow)}
          className="mt-2 px-2 py-1 text-xs rounded text-gray-400 hover:text-white hover:bg-gray-700"
        >
          + {t('documents:sheet.addRow')}
        </button>
      )}
    </div>
  );
}
