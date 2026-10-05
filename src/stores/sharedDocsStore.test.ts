// Unit tests for the shared-documents reducer and selectors: plain function
// calls, no React, no Tauri.

import { describe, expect, it, vi } from 'vitest';
import type { DocFolder, SharedDoc } from '@/types';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

import {
  initialSharedDocsState,
  type SharedDocsState,
  selectDocuments,
  selectFolderDocs,
  selectInvitations,
  sharedDocsReducer,
} from './sharedDocsStore';

function doc(id: string, updatedAt: number, status: SharedDoc['status'] = 'active'): SharedDoc {
  return {
    id,
    accountId: 'acc-1',
    kind: 'doc',
    title: `Doc ${id}`,
    status,
    participants: ['me@example.com'],
    consentedAt: null,
    dirtySince: null,
    createdAt: 0,
    updatedAt,
    folderId: null,
  };
}

function folder(id: string, parentId: string | null = null): DocFolder {
  return { id, accountId: 'acc-1', parentId, name: id, createdAt: 0 };
}

function loaded(docs: SharedDoc[]): SharedDocsState {
  return sharedDocsReducer(
    { ...initialSharedDocsState, accountId: 'acc-1' },
    { type: 'loaded', accountId: 'acc-1', docs },
  );
}

describe('sharedDocsReducer', () => {
  it('drops a list loaded for an account no longer shown', () => {
    const state = sharedDocsReducer(
      { ...initialSharedDocsState, accountId: 'acc-2' },
      { type: 'loaded', accountId: 'acc-1', docs: [doc('a', 1)] },
    );
    expect(state.docs).toEqual([]);
  });

  it('puts a created or changed document first and replaces its old copy', () => {
    let state = loaded([doc('a', 1), doc('b', 2)]);
    state = sharedDocsReducer(state, { type: 'upserted', doc: doc('a', 5) });
    state = sharedDocsReducer(state, { type: 'upserted', doc: doc('c', 3) });
    expect(state.docs.map((d) => [d.id, d.updatedAt])).toEqual([
      ['a', 5],
      ['c', 3],
      ['b', 2],
    ]);
  });

  it('counts each backend change per document so open editors catch up', () => {
    let state = loaded([doc('a', 1)]);
    state = sharedDocsReducer(state, { type: 'changed', docIds: ['a', 'x'] });
    state = sharedDocsReducer(state, { type: 'changed', docIds: ['a'] });
    expect(state.changes).toEqual({ a: 2, x: 1 });
  });

  it('forgets the selection when the account changes', () => {
    let state = sharedDocsReducer(loaded([doc('a', 1)]), { type: 'selected', id: 'a' });
    state = sharedDocsReducer(state, { type: 'accountChanged', accountId: 'acc-2' });
    expect(state.selectedId).toBeNull();
    expect(state.docs).toEqual([]);
  });
});

describe('selectors', () => {
  it('splits invitations from the documents the user works in', () => {
    const state = loaded([doc('a', 3), doc('i', 2, 'invited'), doc('l', 1, 'left')]);
    expect(selectInvitations(state).map((d) => d.id)).toEqual(['i']);
    expect(selectDocuments(state).map((d) => d.id)).toEqual(['a', 'l']);
  });
});

describe('folders and search', () => {
  it('shows the documents of the open folder only', () => {
    let state = loaded([{ ...doc('a', 2), folderId: 'trips' }, doc('b', 1), doc('i', 3, 'invited')]);
    expect(selectFolderDocs(state).map((d) => d.id)).toEqual(['b']);
    state = sharedDocsReducer(state, { type: 'folderOpened', folderId: 'trips' });
    expect(selectFolderDocs(state).map((d) => d.id)).toEqual(['a']);
  });

  it('a deleted open folder sends the view back to its parent', () => {
    let state = sharedDocsReducer(loaded([]), {
      type: 'foldersLoaded',
      folders: [folder('trips'), folder('lisbon', 'trips')],
    });
    state = sharedDocsReducer(state, { type: 'folderOpened', folderId: 'lisbon' });
    state = sharedDocsReducer(state, { type: 'foldersLoaded', folders: [folder('trips')] });
    expect(state.folderId).toBe('trips');
  });

  it('keeps only the answer to the latest search', () => {
    let state = sharedDocsReducer(loaded([]), { type: 'searchStarted', query: 'hot' });
    state = sharedDocsReducer(state, { type: 'searchStarted', query: 'hotel' });
    state = sharedDocsReducer(state, { type: 'searched', query: 'hot', results: [doc('x', 1)] });
    expect(state.searchResults).toBeNull();
    state = sharedDocsReducer(state, { type: 'searched', query: 'hotel', results: [doc('y', 1)] });
    expect(state.searchResults?.map((d) => d.id)).toEqual(['y']);
    state = sharedDocsReducer(state, { type: 'searchStarted', query: '  ' });
    expect(state.searchResults).toBeNull();
  });
});
