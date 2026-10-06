import { describe, expect, it } from 'vitest';
import type { DocFolder } from '@/types';
import { childFolders, folderOptions, folderPath } from './docFolders';

function folder(id: string, parentId: string | null, name: string): DocFolder {
  return { id, accountId: 'acc-1', parentId, name, createdAt: 0 };
}

const FOLDERS = [
  folder('trips', null, 'Trips'),
  folder('lisbon', 'trips', 'Lisbon'),
  folder('receipts', 'lisbon', 'Receipts'),
  folder('admin', null, 'Admin'),
];

describe('folderPath', () => {
  it('lists the folders from the top level down to the given one', () => {
    expect(folderPath(FOLDERS, 'receipts').map((f) => f.name)).toEqual(['Trips', 'Lisbon', 'Receipts']);
    expect(folderPath(FOLDERS, null)).toEqual([]);
    expect(folderPath(FOLDERS, 'missing')).toEqual([]);
  });

  it('stops on a cycle instead of looping', () => {
    const looped = [folder('a', 'b', 'A'), folder('b', 'a', 'B')];
    expect(folderPath(looped, 'a').length).toBeLessThanOrEqual(2);
  });
});

describe('childFolders', () => {
  it('returns the direct children by name', () => {
    expect(childFolders(FOLDERS, null).map((f) => f.name)).toEqual(['Admin', 'Trips']);
    expect(childFolders(FOLDERS, 'trips').map((f) => f.name)).toEqual(['Lisbon']);
  });
});

describe('folderOptions', () => {
  it('lists every folder depth-first with its depth, for a Move to menu', () => {
    expect(folderOptions(FOLDERS).map((o) => `${o.depth}:${o.folder.name}`)).toEqual([
      '0:Admin',
      '0:Trips',
      '1:Lisbon',
      '2:Receipts',
    ]);
  });
});
