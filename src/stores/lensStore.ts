// Lenses store — AI-extracted, schema-typed views over the mailbox.
//
// Backend is the source of truth (SQLite + ai_background queue). This store
// caches the list of Lenses, the currently selected Lens's rows, and run status
// for the badge in the sidebar. Like other stores in this repo, destructure
// reactive fields rather than reading via `getState()` inside memo deps —
// see CLAUDE.md "Zustand Store Subscriptions".
//
// Architecture: pure-reducer + async-action pattern
// -------------------------------------------------
// State transitions are expressed as pure reducer functions that take
// `(state, action) → Partial<LensState>` and can be unit-tested without React
// or Tauri. Async actions call the API and then dispatch into the reducer.
// Selectors are exported alongside the store so tests assert on them rather
// than raw state shape.
//
// To add a new action:
//  1. Add a variant to `LensAction`.
//  2. Handle it in `lensReducer`.
//  3. Write a unit test against `lensReducer` directly (no store setup needed).
//  4. Wrap in a thin async store method if it needs I/O.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { create } from 'zustand';

import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type {
  CreateLensInput,
  Lens,
  LensRow,
  LensRunKind,
  LensSortSpec,
  LensStatus,
  LensSummary,
  UpdateLensInput,
} from '@/types';

// ── State shape ───────────────────────────────────────────────────────────────

export interface LensState {
  lenses: LensSummary[];
  isLoadingLenses: boolean;
  activeLensId: string | null;
  activeLens: Lens | null;
  rows: LensRow[];
  totalRows: number;
  isLoadingRows: boolean;
  sort: LensSortSpec | null;
  error: string | null;
  runStatus: Record<string, LensStatus>;
}

export const initialLensState: LensState = {
  lenses: [],
  isLoadingLenses: false,
  activeLensId: null,
  activeLens: null,
  rows: [],
  totalRows: 0,
  isLoadingRows: false,
  sort: null,
  error: null,
  runStatus: {},
};

// ── Actions ───────────────────────────────────────────────────────────────────

export type LensAction =
  | { type: 'SET_LOADING_LENSES'; loading: boolean }
  | { type: 'SET_LENSES'; lenses: LensSummary[] }
  | { type: 'SET_ERROR'; error: string | null }
  | { type: 'SET_ACTIVE_LENS_ID'; lensId: string | null }
  | { type: 'SET_LOADING_ROWS'; loading: boolean }
  | { type: 'SET_ACTIVE_LENS_DATA'; lens: Lens; rows: LensRow[]; total: number }
  | { type: 'SET_ROWS'; rows: LensRow[]; total: number }
  | { type: 'SET_SORT'; sort: LensSortSpec | null }
  | { type: 'UPDATE_ACTIVE_LENS'; lens: Lens }
  | { type: 'DESELECT_LENS' }
  | { type: 'EXCLUDE_ROW'; emailId: string }
  | { type: 'SET_RUN_STATUS'; lensId: string; status: LensStatus };

// ── Pure reducer ─────────────────────────────────────────────────────────────

/**
 * Pure reducer: (state, action) → next state.
 *
 * No side effects. No async. Unit-testable without React or Tauri.
 */
export function lensReducer(state: LensState, action: LensAction): LensState {
  switch (action.type) {
    case 'SET_LOADING_LENSES':
      return { ...state, isLoadingLenses: action.loading };
    case 'SET_LENSES':
      return { ...state, lenses: action.lenses, isLoadingLenses: false };
    case 'SET_ERROR':
      return { ...state, error: action.error, isLoadingLenses: false, isLoadingRows: false };
    case 'SET_ACTIVE_LENS_ID':
      return { ...state, activeLensId: action.lensId, isLoadingRows: true, error: null };
    case 'SET_LOADING_ROWS':
      return { ...state, isLoadingRows: action.loading };
    case 'SET_ACTIVE_LENS_DATA':
      return {
        ...state,
        activeLens: action.lens,
        rows: action.rows,
        totalRows: action.total,
        isLoadingRows: false,
      };
    case 'SET_ROWS':
      return { ...state, rows: action.rows, totalRows: action.total };
    case 'SET_SORT':
      return { ...state, sort: action.sort };
    case 'UPDATE_ACTIVE_LENS':
      return { ...state, activeLens: action.lens };
    case 'DESELECT_LENS':
      return { ...state, activeLensId: null, activeLens: null, rows: [], totalRows: 0 };
    case 'EXCLUDE_ROW':
      return {
        ...state,
        rows: state.rows.filter((r) => r.emailId !== action.emailId),
        totalRows: Math.max(0, state.totalRows - 1),
      };
    case 'SET_RUN_STATUS':
      return { ...state, runStatus: { ...state.runStatus, [action.lensId]: action.status } };
  }
}

// ── Selectors ─────────────────────────────────────────────────────────────────

/** Active lens's current run status, or undefined if no lens is selected. */
export function selectActiveRunStatus(state: LensState): LensStatus | undefined {
  return state.activeLensId ? state.runStatus[state.activeLensId] : undefined;
}

/** True when the active lens has a run in progress. */
export function selectIsRunning(state: LensState): boolean {
  const status = selectActiveRunStatus(state);
  return status?.state === 'running';
}

/** All lenses sorted by their original order from the backend. */
export function selectLenses(state: LensState): LensSummary[] {
  return state.lenses;
}

// ── Store ─────────────────────────────────────────────────────────────────────

interface LensStore extends LensState {
  // List shown in the sidebar
  lenses: LensSummary[];
  isLoadingLenses: boolean;

  // Active lens detail + rows
  activeLensId: string | null;
  activeLens: Lens | null;
  rows: LensRow[];
  totalRows: number;
  isLoadingRows: boolean;
  sort: LensSortSpec | null;
  error: string | null;

  // Run progress for the active lens (driven by app-log events)
  runStatus: Record<string, LensStatus>;

  // Lifecycle
  initialize: () => Promise<void>;
  startStatusListener: () => Promise<UnlistenFn>;

  // List actions
  refreshLenses: () => Promise<void>;

  // Selection
  selectLens: (lensId: string | null) => Promise<void>;
  setSort: (sort: LensSortSpec | null) => Promise<void>;

  // CRUD
  createLens: (input: CreateLensInput) => Promise<Lens>;
  updateLens: (lensId: string, input: UpdateLensInput) => Promise<Lens>;
  deleteLens: (lensId: string) => Promise<void>;
  duplicateLens: (lensId: string, newName: string) => Promise<Lens>;

  // Runs
  runLens: (lensId: string, kind?: LensRunKind) => Promise<void>;
  cancelRun: (lensId: string) => Promise<void>;
  refreshActiveStatus: () => Promise<void>;

  // Row-level actions
  reextractRow: (emailId: string) => Promise<void>;
  excludeRow: (emailId: string) => Promise<void>;
  updateRowOverride: (emailId: string, overrides: Record<string, unknown>) => Promise<void>;
}

/** Dispatch helper: merges the reducer result into Zustand state. */
function dispatch(set: (fn: (s: LensState) => Partial<LensState>) => void, action: LensAction) {
  set((s) => lensReducer(s, action));
}

export const useLensStore = create<LensStore>((set, get) => ({
  ...initialLensState,

  initialize: async () => {
    await get().refreshLenses();
  },

  startStatusListener: async () => {
    // The backend emits `app-log` events with `source: "lens"` on every step.
    // Whenever we see one we know an active run is mutating state, so refresh
    // the active lens's status and rows. This is cheap because both queries
    // hit indexed columns.
    const unlisten = await listen<{ level: string; source: string; message: string }>('app-log', (e) => {
      if (e.payload.source !== 'lens') return;
      const { activeLensId } = get();
      if (!activeLensId) return;
      // Debounce-ish: fire the refresh; the backend is single-concurrency on
      // ai_background so we won't pile up too many requests.
      void get().refreshActiveStatus();
      if (e.payload.level === 'success' || e.payload.level === 'error') {
        // Run finished (success or failure) — reload rows and lens list so the
        // sidebar badge and "last run" status reflect the final state.
        void get().selectLens(activeLensId);
        void get().refreshLenses();
      }
    });
    return unlisten;
  },

  refreshLenses: async () => {
    dispatch(set, { type: 'SET_LOADING_LENSES', loading: true });
    try {
      const lenses = await api.listLenses();
      dispatch(set, { type: 'SET_LENSES', lenses });
    } catch (err) {
      dispatch(set, { type: 'SET_ERROR', error: errorText(err) });
    }
  },

  selectLens: async (lensId) => {
    if (lensId === null) {
      dispatch(set, { type: 'DESELECT_LENS' });
      return;
    }
    dispatch(set, { type: 'SET_ACTIVE_LENS_ID', lensId });
    try {
      const [lens, page] = await Promise.all([
        api.getLens(lensId),
        api.getLensRows(lensId, { sort: get().sort ?? undefined }),
      ]);
      // Guard against stale fetches if the user clicked another lens mid-flight.
      if (get().activeLensId !== lensId) return;
      dispatch(set, { type: 'SET_ACTIVE_LENS_DATA', lens, rows: page.rows, total: page.total });
      void get().refreshActiveStatus();
    } catch (err) {
      dispatch(set, { type: 'SET_ERROR', error: errorText(err) });
    }
  },

  setSort: async (sort) => {
    dispatch(set, { type: 'SET_SORT', sort });
    const id = get().activeLensId;
    if (id) {
      const page = await api.getLensRows(id, { sort: sort ?? undefined });
      if (get().activeLensId !== id) return;
      dispatch(set, { type: 'SET_ROWS', rows: page.rows, total: page.total });
    }
  },

  createLens: async (input) => {
    const lens = await api.createLens(input);
    await get().refreshLenses();
    return lens;
  },

  updateLens: async (lensId, input) => {
    const lens = await api.updateLens(lensId, input);
    if (get().activeLensId === lensId) {
      dispatch(set, { type: 'UPDATE_ACTIVE_LENS', lens });
    }
    await get().refreshLenses();
    return lens;
  },

  deleteLens: async (lensId) => {
    await api.deleteLens(lensId);
    if (get().activeLensId === lensId) {
      dispatch(set, { type: 'DESELECT_LENS' });
    }
    await get().refreshLenses();
  },

  duplicateLens: async (lensId, newName) => {
    const lens = await api.duplicateLens(lensId, newName);
    await get().refreshLenses();
    return lens;
  },

  runLens: async (lensId, kind) => {
    await api.runLens(lensId, kind);
    // Status will tick via the app-log listener; do an immediate fetch so the
    // UI shows a "running" indicator without waiting for the first event.
    void get().refreshActiveStatus();
  },

  cancelRun: async (lensId) => {
    await api.cancelLensRun(lensId);
    void get().refreshActiveStatus();
  },

  refreshActiveStatus: async () => {
    const id = get().activeLensId;
    if (!id) return;
    try {
      const status = await api.getLensStatus(id);
      dispatch(set, { type: 'SET_RUN_STATUS', lensId: id, status });
    } catch {
      // best-effort
    }
  },

  reextractRow: async (emailId) => {
    const id = get().activeLensId;
    if (!id) return;
    await api.reextractLensRow(id, emailId);
  },

  excludeRow: async (emailId) => {
    const id = get().activeLensId;
    if (!id) return;
    await api.excludeLensRow(id, emailId);
    dispatch(set, { type: 'EXCLUDE_ROW', emailId });
  },

  updateRowOverride: async (emailId, overrides) => {
    const id = get().activeLensId;
    if (!id) return;
    await api.updateLensRowOverride(id, emailId, overrides);
    // Reload the page to pick up the merged values.
    const page = await api.getLensRows(id, { sort: get().sort ?? undefined });
    if (get().activeLensId !== id) return;
    dispatch(set, { type: 'SET_ROWS', rows: page.rows, total: page.total });
  },
}));
