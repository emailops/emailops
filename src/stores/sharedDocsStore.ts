import { listen } from '@tauri-apps/api/event';
import { create } from 'zustand';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { DocKind, SharedDoc } from '@/types';

/**
 * Shared documents of the account the Documents view shows. The list and the
 * selection live here; a document's content lives in its editor's `Y.Doc`
 * (see `useSharedYDoc`). `changes` counts, per document, the backend's
 * `shared-docs-changed` events, so an open editor knows to pull what arrived.
 */
export interface SharedDocsState {
  accountId: string | null;
  docs: SharedDoc[];
  selectedId: string | null;
  isLoading: boolean;
  error: string | null;
  changes: Record<string, number>;
}

export type SharedDocsAction =
  | { type: 'accountChanged'; accountId: string | null }
  | { type: 'loading' }
  | { type: 'loaded'; accountId: string; docs: SharedDoc[] }
  | { type: 'failed'; error: string }
  | { type: 'upserted'; doc: SharedDoc }
  | { type: 'selected'; id: string | null }
  | { type: 'changed'; docIds: string[] };

export const initialSharedDocsState: SharedDocsState = {
  accountId: null,
  docs: [],
  selectedId: null,
  isLoading: false,
  error: null,
  changes: {},
};

function newestFirst(docs: SharedDoc[]): SharedDoc[] {
  return [...docs].sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id));
}

export function sharedDocsReducer(state: SharedDocsState, action: SharedDocsAction): SharedDocsState {
  switch (action.type) {
    case 'accountChanged':
      return { ...initialSharedDocsState, accountId: action.accountId, changes: state.changes };
    case 'loading':
      return { ...state, isLoading: true, error: null };
    case 'loaded':
      // A slow answer for an account the user already switched away from.
      if (action.accountId !== state.accountId) return state;
      return { ...state, isLoading: false, docs: newestFirst(action.docs) };
    case 'failed':
      return { ...state, isLoading: false, error: action.error };
    case 'upserted':
      return { ...state, docs: newestFirst([action.doc, ...state.docs.filter((d) => d.id !== action.doc.id)]) };
    case 'selected':
      return { ...state, selectedId: action.id };
    case 'changed': {
      const changes = { ...state.changes };
      for (const id of action.docIds) changes[id] = (changes[id] ?? 0) + 1;
      return { ...state, changes };
    }
  }
}

export const selectInvitations = (s: SharedDocsState) => s.docs.filter((d) => d.status === 'invited');
export const selectDocuments = (s: SharedDocsState) => s.docs.filter((d) => d.status !== 'invited');
export const selectSelectedDoc = (s: SharedDocsState) => s.docs.find((d) => d.id === s.selectedId) ?? null;

interface SharedDocsStore extends SharedDocsState {
  /** Show `accountId`'s documents (reloads only when it changed). */
  setAccount: (accountId: string | null) => Promise<void>;
  reload: () => Promise<void>;
  select: (id: string | null) => void;
  create: (kind: DocKind, title: string) => Promise<SharedDoc>;
  share: (docId: string, recipients: string[], snapshotHtml: string | null) => Promise<SharedDoc>;
  accept: (docId: string) => Promise<SharedDoc>;
  leave: (docId: string) => Promise<SharedDoc>;
}

export const useSharedDocsStore = create<SharedDocsStore>((set, get) => {
  const dispatch = (action: SharedDocsAction) => set((s) => sharedDocsReducer(s, action));
  const requireAccount = () => {
    const { accountId } = get();
    if (!accountId) throw new Error('No account selected');
    return accountId;
  };
  const upsert = (doc: SharedDoc) => {
    dispatch({ type: 'upserted', doc });
    return doc;
  };

  return {
    ...initialSharedDocsState,
    setAccount: async (accountId) => {
      if (accountId === get().accountId) return;
      dispatch({ type: 'accountChanged', accountId });
      await get().reload();
    },
    reload: async () => {
      const { accountId } = get();
      if (!accountId) return;
      dispatch({ type: 'loading' });
      try {
        dispatch({ type: 'loaded', accountId, docs: await api.listSharedDocs(accountId) });
      } catch (err) {
        dispatch({ type: 'failed', error: errorText(err) });
      }
    },
    select: (id) => dispatch({ type: 'selected', id }),
    create: async (kind, title) => upsert(await api.createSharedDoc(requireAccount(), kind, title)),
    share: async (docId, recipients, snapshotHtml) =>
      upsert(await api.shareSharedDoc(requireAccount(), docId, recipients, snapshotHtml)),
    accept: async (docId) => upsert(await api.acceptSharedDoc(requireAccount(), docId)),
    leave: async (docId) => upsert(await api.leaveSharedDoc(requireAccount(), docId)),
  };
});

// Subscribe once (module scope): a peer's changes or a new invitation arrived
// during a sync. Open editors pull the diff (`changes`), the list reloads.
void listen<{ docIds?: unknown }>('shared-docs-changed', (event) => {
  const ids = event.payload?.docIds;
  if (!Array.isArray(ids)) return;
  const docIds = ids.filter((id): id is string => typeof id === 'string');
  if (docIds.length === 0) return;
  useSharedDocsStore.setState((s) => sharedDocsReducer(s, { type: 'changed', docIds }));
  void useSharedDocsStore.getState().reload();
});
