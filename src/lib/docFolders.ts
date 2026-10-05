import type { DocFolder } from '@/types';

/** Pure helpers over the personal EO Docs folder tree. */

const byName = (a: DocFolder, b: DocFolder) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' });

/** The folders from the top level down to `folderId` (empty for the top level). */
export function folderPath(folders: DocFolder[], folderId: string | null): DocFolder[] {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const path: DocFolder[] = [];
  let current = folderId ? byId.get(folderId) : undefined;
  while (current && !path.includes(current)) {
    path.unshift(current);
    current = current.parentId ? byId.get(current.parentId) : undefined;
  }
  return path;
}

/** The folders directly inside `parentId` (the top level for `null`), by name. */
export function childFolders(folders: DocFolder[], parentId: string | null): DocFolder[] {
  return folders.filter((f) => f.parentId === parentId).sort(byName);
}

export interface FolderOption {
  folder: DocFolder;
  depth: number;
}

/** Every folder, depth-first under its parent, with its depth. */
export function folderOptions(folders: DocFolder[]): FolderOption[] {
  const out: FolderOption[] = [];
  const visit = (parentId: string | null, depth: number) => {
    for (const folder of childFolders(folders, parentId)) {
      if (out.some((o) => o.folder.id === folder.id)) continue;
      out.push({ folder, depth });
      visit(folder.id, depth + 1);
    }
  };
  visit(null, 0);
  return out;
}
