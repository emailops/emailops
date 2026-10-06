import * as Y from 'yjs';
import { readGrid, setCell } from '@/lib/sheetModel';

/**
 * Concurrent edits of one sheet cell: two people changed it before either saw
 * the other's value, and the CRDT kept one of the two — the same one on every
 * copy, but not necessarily the later one. These are the values it dropped.
 *
 * Found from the document itself. A map entry's `origin` is the entry its
 * author had seen when writing it; an entry that replaced one its author never
 * saw replaced it concurrently. This reads Yjs internals (`_map`, `Item`), so
 * the document must keep deleted content (`new Y.Doc({ gc: false })`) and the
 * backend must store updates merged, never re-encoded through a collected doc.
 */
export interface CellConflict {
  /** The dropped entry's id: stable on every copy. */
  id: string;
  rowId: string;
  colId: string;
  row: number;
  col: number;
  /** The value that was dropped. */
  lost: string;
  /** What the cell holds now. */
  kept: string;
  /** Someone already chose which value stays. */
  resolved: boolean;
}

const CELLS = 'cells';
const RESOLVED = 'resolvedConflicts';

type MapItem = Y.Item & { origin: Y.ID | null; left: MapItem | null; content: Y.ContentAny | Y.ContentDeleted };

function itemValue(item: MapItem): string | null {
  if (!(item.content instanceof Y.ContentAny)) return null;
  const [value] = item.content.getContent();
  return typeof value === 'string' ? value : null;
}

const idKey = (id: Y.ID) => `${id.client}:${id.clock}`;

/** Every concurrent overwrite of a cell that still exists, oldest row first. */
export function findConflicts(doc: Y.Doc): CellConflict[] {
  const grid = readGrid(doc);
  const resolved = doc.getMap<boolean>(RESOLVED);
  const entries = (doc.getMap(CELLS) as unknown as { _map: Map<string, MapItem> })._map;
  const found: CellConflict[] = [];
  for (const [key, last] of entries) {
    const [rowId, colId] = key.split(':');
    const row = grid.rowIds.indexOf(rowId);
    const col = grid.colIds.indexOf(colId);
    if (row < 0 || col < 0) continue;
    for (let newer = last; newer.left; newer = newer.left) {
      const older = newer.left;
      const sawOlder = newer.origin !== null && Y.compareIDs(newer.origin, older.id);
      const lost = itemValue(older);
      if (sawOlder || lost === null) continue;
      const id = idKey(older.id);
      found.push({
        id,
        rowId,
        colId,
        row,
        col,
        lost,
        kept: grid.values[row][col],
        resolved: resolved.get(id) === true,
      });
    }
  }
  return found.sort((a, b) => a.row - b.row || a.col - b.col);
}

/** Settle a conflict for everyone: put the dropped value back, or keep what
 *  the cell shows. Either way it is marked resolved in the document. */
export function resolveConflict(doc: Y.Doc, conflict: CellConflict, keep: 'lost' | 'kept'): void {
  doc.transact(() => {
    if (keep === 'lost') setCell(doc, conflict.rowId, conflict.colId, conflict.lost);
    doc.getMap<boolean>(RESOLVED).set(conflict.id, true);
  });
}
