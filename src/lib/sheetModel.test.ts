import { describe, expect, it } from 'vitest';
import * as Y from 'yjs';
import {
  columnLabel,
  DEFAULT_COL_WIDTH,
  deleteColumn,
  deleteRow,
  ensureGrid,
  gridToHtml,
  insertColumn,
  insertRow,
  pasteBlock,
  readGrid,
  setCell,
  setColumnWidth,
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
  it('leaves out the empty rows and columns after the last value', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 5, 4, ids('t'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[1], colIds[1], 'x');
    expect(gridToHtml(readGrid(doc))).toBe(
      '<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td></td><td></td></tr><tr><td></td><td>x</td></tr></tbody></table>',
    );
  });

  it('shows formulas as their results, as the copy is read without EmailOps', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 2, 1, ids('f'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[0], colIds[0], '2');
    setCell(doc, rowIds[1], colIds[0], '=SUM(A1:A1)');
    expect(gridToHtml(readGrid(doc), 'en')).toContain('<td>2</td></tr><tr><td>2</td>');
  });

  it('renders a table with every value escaped', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 1, 2, ids('h'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[0], colIds[0], '<b>&"x"');
    expect(gridToHtml(readGrid(doc))).toBe(
      '<table><thead><tr><th>A</th></tr></thead><tbody><tr><td>&lt;b&gt;&amp;&quot;x&quot;</td></tr></tbody></table>',
    );
  });
});

describe('pasteBlock', () => {
  it('writes the block from the chosen cell, clearing cells pasted empty', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 3, 3, ids('p'));
    const { rowIds, colIds } = readGrid(doc);
    setCell(doc, rowIds[2], colIds[2], 'old');
    pasteBlock(doc, 1, 1, [
      ['a', 'b'],
      ['c', ''],
    ]);
    expect(readGrid(doc).values).toEqual([
      ['', '', ''],
      ['', 'a', 'b'],
      ['', 'c', ''],
    ]);
  });

  it('adds the rows and columns a large block needs', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 2, 2, ids('q'));
    pasteBlock(
      doc,
      1,
      1,
      [
        ['x', 'y', 'z'],
        ['1', '2', '3'],
      ],
      ids('n'),
    );
    const grid = readGrid(doc);
    expect(grid.rowIds).toHaveLength(3);
    expect(grid.colIds).toHaveLength(4);
    expect(grid.values[2]).toEqual(['', '1', '2', '3']);
  });

  it('is one change, so a peer receives the whole block at once', () => {
    const doc = new Y.Doc();
    ensureGrid(doc, 1, 1, ids('t'));
    let updates = 0;
    doc.on('update', () => {
      updates += 1;
    });
    pasteBlock(
      doc,
      0,
      0,
      [
        ['a', 'b'],
        ['c', 'd'],
      ],
      ids('u'),
    );
    expect(updates).toBe(1);
  });
});

describe('column widths', () => {
  it('default, then follow a resize, clamped to a usable size, and travel to peers', () => {
    const a = new Y.Doc();
    ensureGrid(a, 1, 2, ids('w'));
    const [first, second] = readGrid(a).colIds;
    expect(readGrid(a).widths).toEqual([DEFAULT_COL_WIDTH, DEFAULT_COL_WIDTH]);
    setColumnWidth(a, first, 250);
    setColumnWidth(a, second, 5);
    expect(readGrid(a).widths).toEqual([250, 40]);
    const b = new Y.Doc();
    Y.applyUpdate(b, Y.encodeStateAsUpdate(a));
    expect(readGrid(b).widths).toEqual([250, 40]);
  });
});
