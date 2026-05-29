// Unit tests for the lensStore pure reducer and selectors.
//
// No React, no Tauri, no Zustand — just plain function calls.
// These are the tests that catch state-transition bugs without needing
// a full component tree.

import { describe, expect, it } from 'vitest';

import type { LensRow, LensStatus, LensSummary } from '@/types';
import {
  initialLensState,
  type LensAction,
  type LensState,
  lensReducer,
  selectActiveRunStatus,
  selectIsRunning,
  selectLenses,
} from './lensStore';

// ── Helpers ────────────────────────────────────────────────────────────────

function makeSummary(id: string): LensSummary {
  return {
    id,
    name: `Lens ${id}`,
    icon: null,
    templateKey: null,
    accountId: null,
    isEnabled: true,
    sortOrder: 0,
    rowCount: 0,
    staleCount: 0,
  };
}

function makeRow(emailId: string): LensRow {
  return {
    lensId: 'lens-1',
    emailId,
    accountId: 'acc-1',
    emailSubject: 'Test',
    emailSender: 'sender@test.com',
    emailTimestamp: 1000,
    data: {},
    hasOverrides: false,
    promptVersion: 1,
    status: 'ok',
    errorMessage: null,
    extractedAt: 1000,
  };
}

function makeStatus(state: 'idle' | 'running' | 'error' = 'idle'): LensStatus {
  return {
    lensId: 'lens-1',
    state,
    currentRunId: null,
    currentRunKind: null,
    processed: 0,
    total: 0,
    succeeded: 0,
    failed: 0,
    pendingReextract: 0,
    lastError: null,
  };
}

function reduce(state: LensState, action: LensAction): LensState {
  return lensReducer(state, action);
}

// ── SET_LOADING_LENSES ─────────────────────────────────────────────────────

describe('SET_LOADING_LENSES', () => {
  it('sets isLoadingLenses', () => {
    const s = reduce(initialLensState, { type: 'SET_LOADING_LENSES', loading: true });
    expect(s.isLoadingLenses).toBe(true);
  });

  it('clears isLoadingLenses', () => {
    const s0: LensState = { ...initialLensState, isLoadingLenses: true };
    const s = reduce(s0, { type: 'SET_LOADING_LENSES', loading: false });
    expect(s.isLoadingLenses).toBe(false);
  });
});

// ── SET_LENSES ─────────────────────────────────────────────────────────────

describe('SET_LENSES', () => {
  it('replaces the list and clears loading flag', () => {
    const s0: LensState = { ...initialLensState, isLoadingLenses: true };
    const lenses = [makeSummary('a'), makeSummary('b')];
    const s = reduce(s0, { type: 'SET_LENSES', lenses });
    expect(s.lenses).toEqual(lenses);
    expect(s.isLoadingLenses).toBe(false);
  });
});

// ── SET_ERROR ──────────────────────────────────────────────────────────────

describe('SET_ERROR', () => {
  it('stores the error message and clears loading flags', () => {
    const s0: LensState = { ...initialLensState, isLoadingLenses: true, isLoadingRows: true };
    const s = reduce(s0, { type: 'SET_ERROR', error: 'boom' });
    expect(s.error).toBe('boom');
    expect(s.isLoadingLenses).toBe(false);
    expect(s.isLoadingRows).toBe(false);
  });
});

// ── DESELECT_LENS ──────────────────────────────────────────────────────────

describe('DESELECT_LENS', () => {
  it('clears active lens, rows, and total', () => {
    const s0: LensState = {
      ...initialLensState,
      activeLensId: 'lens-1',
      rows: [makeRow('e1')],
      totalRows: 1,
    };
    const s = reduce(s0, { type: 'DESELECT_LENS' });
    expect(s.activeLensId).toBeNull();
    expect(s.rows).toHaveLength(0);
    expect(s.totalRows).toBe(0);
  });
});

// ── EXCLUDE_ROW ────────────────────────────────────────────────────────────

describe('EXCLUDE_ROW', () => {
  it('removes the row and decrements totalRows', () => {
    const s0: LensState = {
      ...initialLensState,
      rows: [makeRow('e1'), makeRow('e2')],
      totalRows: 2,
    };
    const s = reduce(s0, { type: 'EXCLUDE_ROW', emailId: 'e1' });
    expect(s.rows.map((r) => r.emailId)).toEqual(['e2']);
    expect(s.totalRows).toBe(1);
  });

  it('does not go below zero', () => {
    const s0: LensState = { ...initialLensState, rows: [], totalRows: 0 };
    const s = reduce(s0, { type: 'EXCLUDE_ROW', emailId: 'missing' });
    expect(s.totalRows).toBe(0);
  });

  it('keeps other rows untouched', () => {
    const s0: LensState = {
      ...initialLensState,
      rows: [makeRow('e1'), makeRow('e2'), makeRow('e3')],
      totalRows: 3,
    };
    const s = reduce(s0, { type: 'EXCLUDE_ROW', emailId: 'e2' });
    expect(s.rows.map((r) => r.emailId)).toEqual(['e1', 'e3']);
    expect(s.totalRows).toBe(2);
  });
});

// ── SET_RUN_STATUS ─────────────────────────────────────────────────────────

describe('SET_RUN_STATUS', () => {
  it('merges without replacing other statuses', () => {
    const s0: LensState = {
      ...initialLensState,
      runStatus: { 'lens-2': makeStatus('idle') },
    };
    const status = makeStatus('running');
    const s = reduce(s0, { type: 'SET_RUN_STATUS', lensId: 'lens-1', status });
    expect(s.runStatus['lens-1']).toEqual(status);
    expect(s.runStatus['lens-2']).toBeDefined();
  });
});

// ── Selectors ──────────────────────────────────────────────────────────────

describe('selectActiveRunStatus', () => {
  it('returns status for the active lens', () => {
    const status = makeStatus('running');
    const s: LensState = {
      ...initialLensState,
      activeLensId: 'lens-1',
      runStatus: { 'lens-1': status },
    };
    expect(selectActiveRunStatus(s)).toEqual(status);
  });

  it('returns undefined when no lens selected', () => {
    expect(selectActiveRunStatus(initialLensState)).toBeUndefined();
  });
});

describe('selectIsRunning', () => {
  it('true when active lens state is running', () => {
    const s: LensState = {
      ...initialLensState,
      activeLensId: 'lens-1',
      runStatus: { 'lens-1': makeStatus('running') },
    };
    expect(selectIsRunning(s)).toBe(true);
  });

  it('false when active lens state is idle', () => {
    const s: LensState = {
      ...initialLensState,
      activeLensId: 'lens-1',
      runStatus: { 'lens-1': makeStatus('idle') },
    };
    expect(selectIsRunning(s)).toBe(false);
  });

  it('false when no lens is selected', () => {
    expect(selectIsRunning(initialLensState)).toBe(false);
  });
});

describe('selectLenses', () => {
  it('returns the lenses array', () => {
    const lenses = [makeSummary('x')];
    const s: LensState = { ...initialLensState, lenses };
    expect(selectLenses(s)).toBe(lenses);
  });
});
