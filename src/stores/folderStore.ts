import { create } from 'zustand';
import type { Folder } from '@/lib/api';
import * as api from '@/lib/api';

/**
 * Custom IMAP folders of the currently selected account, shown in the
 * sidebar "Folders" section. Folders are account-specific, so the unified
 * ("All accounts") view always carries an empty list.
 */
interface FolderStore {
  folders: Folder[];
  /** Account the current `folders` belong to; null for the unified view. */
  accountId: string | null;
  /** Monotonic token so a slow response for a previous account is dropped. */
  fetchSeq: number;
  fetchFolders: (accountId: string | null) => Promise<void>;
  /** Create/rename/delete throw on failure (callers surface the error) and
   *  refresh the folder list on success. */
  createFolder: (accountId: string, name: string) => Promise<void>;
  /** Resolves to the renamed folder so callers can follow it (the folder id
   *  and server path both change with the name). */
  renameFolder: (accountId: string, folderId: string, newName: string) => Promise<Folder>;
  deleteFolder: (accountId: string, folderId: string) => Promise<void>;
}

export const useFolderStore = create<FolderStore>((set, get) => ({
  folders: [],
  accountId: null,
  fetchSeq: 0,

  fetchFolders: async (accountId) => {
    const seq = get().fetchSeq + 1;
    set({ fetchSeq: seq });

    if (accountId === null) {
      set({ folders: [], accountId: null });
      return;
    }

    try {
      const folders = await api.getFolders(accountId);
      if (get().fetchSeq !== seq) return; // account switched mid-fetch
      set({ folders, accountId });
    } catch {
      if (get().fetchSeq !== seq) return;
      // Backend hiccup: show no folders rather than another account's list.
      // Non-fatal — the section simply hides until the next sync/refresh.
      set({ folders: [], accountId });
    }
  },

  createFolder: async (accountId, name) => {
    await api.createFolder(accountId, name);
    await get().fetchFolders(accountId);
  },

  renameFolder: async (accountId, folderId, newName) => {
    const renamed = await api.renameFolder(accountId, folderId, newName);
    await get().fetchFolders(accountId);
    return renamed;
  },

  deleteFolder: async (accountId, folderId) => {
    await api.deleteFolder(accountId, folderId);
    await get().fetchFolders(accountId);
  },
}));
