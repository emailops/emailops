import { describe, expect, it } from 'vitest';
import * as Y from 'yjs';
import { findConflicts, resolveConflict } from './sheetConflicts';
import { ensureGrid, readGrid, setCell } from './sheetModel';

/** Alice and Bob each hold a copy of the same 3×2 sheet. Client ids are fixed
 *  so which concurrent value wins is the same on every run. */
function pair() {
  const alice = new Y.Doc({ gc: false });
  alice.clientID = 1;
  let n = 0;
  ensureGrid(alice, 3, 2, () => `id${n++}`);
  const bob = new Y.Doc({ gc: false });
  bob.clientID = 2;
  Y.applyUpdate(bob, Y.encodeStateAsUpdate(alice));
  return { alice, bob };
}

const sync = (a: Y.Doc, b: Y.Doc) => {
  Y.applyUpdate(b, Y.encodeStateAsUpdate(a, Y.encodeStateVector(b)));
  Y.applyUpdate(a, Y.encodeStateAsUpdate(b, Y.encodeStateVector(a)));
};

const cellB2 = (doc: Y.Doc) => {
  const { rowIds, colIds } = readGrid(doc);
  return [rowIds[1], colIds[1]] as const;
};

describe('findConflicts', () => {
  it('finds the value one person lost when both changed the same cell before seeing each other', () => {
    const { alice, bob } = pair();
    setCell(alice, ...cellB2(alice), '120');
    setCell(bob, ...cellB2(bob), '130');
    sync(alice, bob);

    const kept = readGrid(alice).values[1][1];
    const lost = kept === '120' ? '130' : '120';
    for (const doc of [alice, bob]) {
      expect(findConflicts(doc)).toEqual([expect.objectContaining({ row: 1, col: 1, lost, kept, resolved: false })]);
    }
  });

  it('is nothing when a cell is changed after its earlier value was seen', () => {
    const { alice, bob } = pair();
    setCell(alice, ...cellB2(alice), '120');
    sync(alice, bob);
    setCell(bob, ...cellB2(bob), '130');
    sync(alice, bob);

    expect(findConflicts(alice)).toEqual([]);
  });

  it('finds the conflict in a state merged from the separate updates, as the backend stores it', () => {
    const { alice, bob } = pair();
    const base = Y.encodeStateAsUpdate(alice);
    const before = Y.encodeStateVector(alice);
    setCell(alice, ...cellB2(alice), '120');
    setCell(bob, ...cellB2(bob), '130');
    const stored = Y.mergeUpdates([base, Y.encodeStateAsUpdate(alice, before), Y.encodeStateAsUpdate(bob, before)]);

    const opened = new Y.Doc({ gc: false });
    Y.applyUpdate(opened, stored);
    expect(findConflicts(opened)).toHaveLength(1);
  });
});

describe('resolveConflict', () => {
  it('brings the lost value back, and the conflict is resolved for everyone', () => {
    const { alice, bob } = pair();
    setCell(alice, ...cellB2(alice), '120');
    setCell(bob, ...cellB2(bob), '130');
    sync(alice, bob);
    const [conflict] = findConflicts(alice);

    resolveConflict(alice, conflict, 'lost');
    sync(alice, bob);

    expect(readGrid(bob).values[1][1]).toBe(conflict.lost);
    expect(findConflicts(bob)).toEqual([expect.objectContaining({ resolved: true })]);
  });

  it('keeps the value shown when that is the one chosen', () => {
    const { alice, bob } = pair();
    setCell(alice, ...cellB2(alice), '120');
    setCell(bob, ...cellB2(bob), '130');
    sync(alice, bob);
    const [conflict] = findConflicts(alice);

    resolveConflict(alice, conflict, 'kept');

    expect(readGrid(alice).values[1][1]).toBe(conflict.kept);
    expect(findConflicts(alice)[0].resolved).toBe(true);
  });
});
