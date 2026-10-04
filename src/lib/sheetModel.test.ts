import { describe, expect, it } from 'vitest';
import * as Y from 'yjs';
import {
  columnLabel,
  deleteColumn,
  deleteRow,
  ensureGrid,
  gridToHtml,
  insertColumn,
  insertRow,
  readGrid,
  setCell,
} from './sheetModel';

function ids(prefix: string) {
  let n = 0;
  return () => `${prefix}${n++}`;
}

function sync(a: Y.Doc, b: Y.Doc) {
  Y.applyUpdate(b, Y.encodeStateAsUpdate(a));
  Y.applyUpdate(a, Y.encodeStateAsUpdate(b));
}

describe('ensureGrid', () => {
  it('lays out an empty sheet once and leaves an existing one alone', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 3, 2, ids('x'));
    ensureGrid(doc, 9, 9, ids('y'));
    const grid = readGrid(doc);
    expect(grid.rowIds).toEqual(['x0', 'x1', 'x2']);
    expect(grid.colIds).toEqual(['x3', 'x4']);
    expect(grid.values).toEqual([
      ['', ''],
      ['', ''],
      ['', ''],
    ]);
  });
});

describe('cells', () => {
  it('keeps a value and forgets an emptied one', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 2, 2, ids('r'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[1], colIds[0], 'Total');
    setCell(doc, rowIds[0], colIds[1], '42');
    setCell(doc, rowIds[0], colIds[1], '');
    expect(readGrid(doc).values).toEqual([
      ['', ''],
      ['Total', ''],
    ]);
    expect(doc.getMap('cells').size).toBe(1);
  });
});

describe('rows and columns', () => {
  it('inserting a row keeps every value under its own row', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 2, 1, ids('a'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[0], colIds[0], 'first');
    setCell(doc, rowIds[1], colIds[0], 'second');
    insertRow(doc, 1, () => 'new');
    expect(readGrid(doc).values).toEqual([['first'], [''], ['second']]);
  });

  it('deleting a row or column drops its cells', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 2, 2, ids('a'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[0], colIds[0], 'gone');
    setCell(doc, rowIds[1], colIds[1], 'kept');
    deleteRow(doc, 0);
    insertColumn(doc, 0, () => 'c-new');
    deleteColumn(doc, 0);
    expect(readGrid(doc).values).toEqual([['', 'kept']]);
    expect(Array.from(doc.getMap('cells').keys())).toEqual([`${rowIds[1]}:${colIds[1]}`]);
  });

  it('two people inserting rows at once keep both rows and both values', () => {
    const a = new Y.Doc();
    const b = new Y.Doc();
    ensureGrid(a, 1, 1, ids('s'));
    sync(a, b);
    insertRow(a, 1, () => 'from-a');
    setCell(a, 'from-a', readGrid(a).colIds[0], 'A');
    insertRow(b, 1, () => 'from-b');
    setCell(b, 'from-b', readGrid(b).colIds[0], 'B');
    sync(a, b);
    expect(readGrid(a)).toEqual(readGrid(b));
    expect(
      readGrid(a)
        .values.flat()
        .filter((v) => v)
        .sort(),
    ).toEqual(['A', 'B']);
  });
});

describe('columnLabel', () => {
  it('counts like a spreadsheet', () => {
    expect([0, 1, 25, 26, 27, 51, 52, 701, 702].map(columnLabel)).toEqual([
      'A',
      'B',
      'Z',
      'AA',
      'AB',
      'AZ',
      'BA',
      'ZZ',
      'AAA',
    ]);
  });
});

describe('gridToHtml', () => {
  it('renders a table with every value escaped', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 1, 2, ids('h'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[0], colIds[0], '<b>&"x"');
    expect(gridToHtml(readGrid(doc))).toBe(
      '<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>&lt;b&gt;&amp;&quot;x&quot;</td><td></td></tr></tbody></table>',
    );
  });
});
