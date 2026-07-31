import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Folder } from '@/lib/api';

vi.mock('@/lib/api', () => ({
  getFolders: vi.fn(),
  createFolder: vi.fn(),
  renameFolder: vi.fn(),
  deleteFolder: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import * as api from '@/lib/api';
import { useFolderStore } from './folderStore';

function makeFolder(serverPath: string): Folder {
  return {
    id: `acc-1:${serverPath}`,
    accountId: 'acc-1',
    serverPath,
    displayName: serverPath,
    role: 'custom',
    delimiter: '.',
  };
}

describe('folderStore', () => {
  beforeEach(() => {
    useFolderStore.setState({ folders: [], accountId: null });
    vi.mocked(api.getFolders).mockReset();
  });

  it('fetches and stores the folders of the requested account', async () => {
    vi.mocked(api.getFolders).mockResolvedValue([makeFolder('Patienten')]);

    await useFolderStore.getState().fetchFolders('acc-1');

    const { folders, accountId } = useFolderStore.getState();
    expect(accountId).toBe('acc-1');
    expect(folders.map((f) => f.serverPath)).toEqual(['Patienten']);
  });

  it('clears folders when fetching the unified view (null account)', async () => {
    useFolderStore.setState({ folders: [makeFolder('Alt')], accountId: 'acc-1' });

    await useFolderStore.getState().fetchFolders(null);

    expect(useFolderStore.getState().folders).toEqual([]);
    expect(useFolderStore.getState().accountId).toBeNull();
    expect(api.getFolders).not.toHaveBeenCalled();
  });

  it('ignores a stale response after the account switched mid-fetch', async () => {
    let resolveFirst: (folders: Folder[]) => void = () => {};
    vi.mocked(api.getFolders).mockImplementationOnce(
      () =>
        new Promise<Folder[]>((resolve) => {
          resolveFirst = resolve;
        }),
    );
    const slowFetch = useFolderStore.getState().fetchFolders('acc-1');

    vi.mocked(api.getFolders).mockResolvedValueOnce([makeFolder('B-Folder')]);
    await useFolderStore.getState().fetchFolders('acc-2');

    // The slow acc-1 response lands after acc-2 is active — must be dropped.
    resolveFirst([makeFolder('A-Folder')]);
    await slowFetch;

    const { folders, accountId } = useFolderStore.getState();
    expect(accountId).toBe('acc-2');
    expect(folders.map((f) => f.serverPath)).toEqual(['B-Folder']);
  });

  it('createFolder calls the API then refreshes the folder list', async () => {
    vi.mocked(api.createFolder).mockResolvedValue(makeFolder('Neu'));
    vi.mocked(api.getFolders).mockResolvedValue([makeFolder('Neu')]);

    await useFolderStore.getState().createFolder('acc-1', 'Neu');

    expect(api.createFolder).toHaveBeenCalledWith('acc-1', 'Neu');
    expect(useFolderStore.getState().folders.map((f) => f.serverPath)).toEqual(['Neu']);
  });

  it('createFolder propagates API errors without refreshing', async () => {
    vi.mocked(api.createFolder).mockRejectedValue(new Error('duplicate'));

    await expect(useFolderStore.getState().createFolder('acc-1', 'Neu')).rejects.toThrow('duplicate');
    expect(api.getFolders).not.toHaveBeenCalled();
  });

  it('renameFolder calls the API then refreshes the folder list', async () => {
    useFolderStore.setState({ folders: [makeFolder('Alt')], accountId: 'acc-1' });
    vi.mocked(api.renameFolder).mockResolvedValue(makeFolder('Neu'));
    vi.mocked(api.getFolders).mockResolvedValue([makeFolder('Neu')]);

    const renamed = await useFolderStore.getState().renameFolder('acc-1', 'acc-1:Alt', 'Neu');

    expect(api.renameFolder).toHaveBeenCalledWith('acc-1', 'acc-1:Alt', 'Neu');
    expect(renamed.serverPath).toBe('Neu');
    expect(useFolderStore.getState().folders.map((f) => f.serverPath)).toEqual(['Neu']);
  });

  it('deleteFolder calls the API then refreshes the folder list', async () => {
    useFolderStore.setState({ folders: [makeFolder('Alt')], accountId: 'acc-1' });
    vi.mocked(api.deleteFolder).mockResolvedValue(undefined);
    vi.mocked(api.getFolders).mockResolvedValue([]);

    await useFolderStore.getState().deleteFolder('acc-1', 'acc-1:Alt');

    expect(api.deleteFolder).toHaveBeenCalledWith('acc-1', 'acc-1:Alt');
    expect(useFolderStore.getState().folders).toEqual([]);
  });

  it('keeps an empty list when the backend errors (no stale carryover)', async () => {
    useFolderStore.setState({ folders: [makeFolder('Alt')], accountId: 'acc-old' });
    vi.mocked(api.getFolders).mockRejectedValue(new Error('db closed'));

    await useFolderStore.getState().fetchFolders('acc-1');

    expect(useFolderStore.getState().folders).toEqual([]);
    expect(useFolderStore.getState().accountId).toBe('acc-1');
  });
});
