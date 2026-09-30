// lensStore async actions: the thin I/O layer over the pure reducer
// (lensStore.test.ts covers the reducer itself).
//
// What these pin: stale answers for a Lens the user already left are dropped,
// the `app-log` listener refreshes only on lens events, row actions need an
// active Lens, and CRUD keeps the sidebar list and the open Lens in step.

import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Lens, LensRow, LensStatus, LensSummary } from '@/types';

type AppLog = (event: { payload: { level: string; source: string; message: string } }) => void;
const subscription = vi.hoisted(() => ({ handler: null as AppLog | null }));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((_name: string, handler: AppLog) => {
    subscription.handler = handler;
    return Promise.resolve(() => {});
  }),
}));

const api = vi.hoisted(() => ({
  listLenses: vi.fn(),
  getLens: vi.fn(),
  getLensRows: vi.fn(),
  getExcludedLensRows: vi.fn(),
  getLensStatus: vi.fn(),
  createLens: vi.fn(),
  updateLens: vi.fn(),
  deleteLens: vi.fn(),
  duplicateLens: vi.fn(),
  runLens: vi.fn(),
  cancelLensRun: vi.fn(),
  reextractLensRow: vi.fn(),
  excludeLensRow: vi.fn(),
  includeLensRow: vi.fn(),
  updateLensRowOverride: vi.fn(),
}));

vi.mock('@/lib/api', () => api);

import { initialLensState, useLensStore } from './lensStore';

function summary(id: string): LensSummary {
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

function lens(id: string, name = `Lens ${id}`): Lens {
  return {
    id,
    name,
    icon: null,
    templateKey: null,
    accountId: null,
    scope: {} as Lens['scope'],
    schema: {} as Lens['schema'],
    promptText: '',
    promptVersion: 1,
    modelProvider: null,
    modelName: null,
    isEnabled: true,
    sortOrder: 0,
    createdAt: 0,
    updatedAt: 0,
  };
}

function row(emailId: string): LensRow {
  return {
    lensId: 'lens-1',
    emailId,
    accountId: 'acc-1',
    emailSubject: 'Invoice',
    emailSender: 'billing@example.com',
    emailTimestamp: 0,
    data: {},
    hasOverrides: false,
    promptVersion: 1,
    status: 'ok',
    errorMessage: null,
    extractedAt: 0,
  };
}

function status(lensId: string, state = 'idle'): LensStatus {
  return {
    lensId,
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

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  for (const fn of Object.values(api)) fn.mockReset();
  api.listLenses.mockResolvedValue([summary('lens-1')]);
  api.getLens.mockImplementation((id: string) => Promise.resolve(lens(id)));
  api.getLensRows.mockResolvedValue({ rows: [row('e1'), row('e2')], total: 2 });
  api.getExcludedLensRows.mockResolvedValue({ rows: [row('x1')], total: 1 });
  api.getLensStatus.mockImplementation((id: string) => Promise.resolve(status(id)));
  for (const fn of [api.deleteLens, api.runLens, api.cancelLensRun, api.reextractLensRow])
    fn.mockResolvedValue(undefined);
  for (const fn of [api.excludeLensRow, api.includeLensRow, api.updateLensRowOverride]) fn.mockResolvedValue(undefined);
  useLensStore.setState({ ...initialLensState });
});

describe('lensStore.selectLens', () => {
  it('loads the Lens, its rows and its run status', async () => {
    await useLensStore.getState().selectLens('lens-1');
    await settle();

    const s = useLensStore.getState();
    expect(s.activeLens?.id).toBe('lens-1');
    expect(s.rows.map((r) => r.emailId)).toEqual(['e1', 'e2']);
    expect(s.totalRows).toBe(2);
    expect(s.isLoadingRows).toBe(false);
    expect(s.runStatus['lens-1']?.state).toBe('idle');
  });

  it('drops the answer for a Lens the user clicked away from', async () => {
    const slow = deferred<Lens>();
    api.getLens.mockReturnValueOnce(slow.promise);

    const first = useLensStore.getState().selectLens('lens-a');
    await useLensStore.getState().selectLens('lens-b');
    slow.resolve(lens('lens-a'));
    await first;

    expect(useLensStore.getState().activeLens?.id).toBe('lens-b');
  });

  it('deselects without fetching when given null', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', activeLens: lens('lens-1'), rows: [row('e1')] });

    await useLensStore.getState().selectLens(null);

    expect(useLensStore.getState().activeLensId).toBeNull();
    expect(useLensStore.getState().rows).toEqual([]);
    expect(api.getLens).not.toHaveBeenCalled();
  });

  it('stops the row spinner and keeps the error when loading fails', async () => {
    api.getLensRows.mockRejectedValue(new Error('no such lens'));

    await useLensStore.getState().selectLens('lens-1');

    expect(useLensStore.getState().isLoadingRows).toBe(false);
    expect(useLensStore.getState().error).toContain('no such lens');
  });
});

describe('lensStore list and CRUD', () => {
  it('records a failed list refresh as an error and stops loading', async () => {
    api.listLenses.mockRejectedValue(new Error('db busy'));

    await useLensStore.getState().refreshLenses();

    expect(useLensStore.getState().isLoadingLenses).toBe(false);
    expect(useLensStore.getState().error).toContain('db busy');
  });

  it('refreshes the sidebar list after create and duplicate', async () => {
    api.createLens.mockResolvedValue(lens('new'));
    api.duplicateLens.mockResolvedValue(lens('copy'));

    await useLensStore.getState().createLens({} as never);
    await useLensStore.getState().duplicateLens('lens-1', 'Copy');

    expect(api.duplicateLens).toHaveBeenCalledWith('lens-1', 'Copy');
    expect(api.listLenses).toHaveBeenCalledTimes(2);
  });

  it('updates the open Lens in place, but not a Lens that is not open', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', activeLens: lens('lens-1') });
    api.updateLens.mockResolvedValueOnce(lens('lens-1', 'Renamed'));
    await useLensStore.getState().updateLens('lens-1', {} as never);
    expect(useLensStore.getState().activeLens?.name).toBe('Renamed');

    api.updateLens.mockResolvedValueOnce(lens('lens-2', 'Other'));
    await useLensStore.getState().updateLens('lens-2', {} as never);
    expect(useLensStore.getState().activeLens?.name).toBe('Renamed');
  });

  it('closes the Lens being deleted, and only that one', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', activeLens: lens('lens-1') });
    await useLensStore.getState().deleteLens('lens-2');
    expect(useLensStore.getState().activeLensId).toBe('lens-1');

    await useLensStore.getState().deleteLens('lens-1');
    expect(useLensStore.getState().activeLensId).toBeNull();
    expect(api.listLenses).toHaveBeenCalledTimes(2);
  });
});

describe('lensStore runs and status', () => {
  it('fetches the status right after starting or cancelling a run', async () => {
    useLensStore.setState({ activeLensId: 'lens-1' });
    api.getLensStatus.mockResolvedValueOnce(status('lens-1', 'running'));

    await useLensStore.getState().runLens('lens-1', 'backfill');
    await settle();
    expect(api.runLens).toHaveBeenCalledWith('lens-1', 'backfill');
    expect(useLensStore.getState().runStatus['lens-1']?.state).toBe('running');

    await useLensStore.getState().cancelRun('lens-1');
    await settle();
    expect(api.cancelLensRun).toHaveBeenCalledWith('lens-1');
    expect(useLensStore.getState().runStatus['lens-1']?.state).toBe('idle');
  });

  it('keeps the last known status when the status query fails', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', runStatus: { 'lens-1': status('lens-1', 'running') } });
    api.getLensStatus.mockRejectedValue(new Error('busy'));

    await useLensStore.getState().refreshActiveStatus();

    expect(useLensStore.getState().runStatus['lens-1']?.state).toBe('running');
    expect(useLensStore.getState().error).toBeNull();
  });
});

describe('lensStore app-log listener', () => {
  function log(level: string, source: string) {
    if (!subscription.handler) throw new Error('listener not started');
    subscription.handler({ payload: { level, source, message: 'step' } });
  }

  it('ignores other sources and events while no Lens is open', async () => {
    await useLensStore.getState().startStatusListener();
    log('info', 'sync');
    log('info', 'lens');
    await settle();

    expect(api.getLensStatus).not.toHaveBeenCalled();
  });

  it('refreshes the status on progress and reloads rows and list when a run ends', async () => {
    await useLensStore.getState().startStatusListener();
    useLensStore.setState({ activeLensId: 'lens-1' });

    log('info', 'lens');
    await settle();
    expect(api.getLensStatus).toHaveBeenCalledWith('lens-1');
    expect(api.getLensRows).not.toHaveBeenCalled();

    for (const level of ['success', 'error']) {
      api.getLensRows.mockClear();
      api.listLenses.mockClear();
      log(level, 'lens');
      await settle();
      expect(api.getLensRows).toHaveBeenCalledTimes(1);
      expect(api.listLenses).toHaveBeenCalledTimes(1);
    }
  });
});

describe('lensStore rows', () => {
  it('row actions do nothing without an open Lens', async () => {
    const s = useLensStore.getState();
    await s.reextractRow('e1');
    await s.excludeRow('e1');
    await s.includeRow('e1');
    await s.updateRowOverride('e1', { total: 1 });

    expect(api.reextractLensRow).not.toHaveBeenCalled();
    expect(api.excludeLensRow).not.toHaveBeenCalled();
    expect(api.includeLensRow).not.toHaveBeenCalled();
    expect(api.updateLensRowOverride).not.toHaveBeenCalled();
  });

  it('removes an excluded or re-included row from the list shown', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', rows: [row('e1'), row('e2')], totalRows: 2 });

    await useLensStore.getState().excludeRow('e1');
    expect(api.excludeLensRow).toHaveBeenCalledWith('lens-1', 'e1');
    expect(useLensStore.getState().rows.map((r) => r.emailId)).toEqual(['e2']);

    await useLensStore.getState().includeRow('e2');
    expect(api.includeLensRow).toHaveBeenCalledWith('lens-1', 'e2');
    expect(useLensStore.getState().totalRows).toBe(0);
  });

  it('re-extracts a row of the open Lens', async () => {
    useLensStore.setState({ activeLensId: 'lens-1' });

    await useLensStore.getState().reextractRow('e1');

    expect(api.reextractLensRow).toHaveBeenCalledWith('lens-1', 'e1');
  });

  it('reloads the page with the current sort and filters after an edited cell', async () => {
    const filters = [{ key: 'vendor', values: ['Acme'], includeEmpty: false }];
    const sort = { columnKey: 'total', direction: 'desc' as const };
    useLensStore.setState({ activeLensId: 'lens-1', columnFilters: filters, sort });
    api.getLensRows.mockResolvedValue({ rows: [row('e9')], total: 1 });

    await useLensStore.getState().updateRowOverride('e9', { total: 12 });

    expect(api.updateLensRowOverride).toHaveBeenCalledWith('lens-1', 'e9', { total: 12 });
    expect(api.getLensRows).toHaveBeenCalledWith('lens-1', { sort, filters });
    expect(useLensStore.getState().rows.map((r) => r.emailId)).toEqual(['e9']);
  });

  it('sorts the open Lens and drops a sorted page for a Lens the user left', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', rows: [] });
    const slow = deferred<{ rows: LensRow[]; total: number }>();
    api.getLensRows.mockReturnValueOnce(slow.promise);

    const sorting = useLensStore.getState().setSort({ columnKey: 'total', direction: 'asc' });
    useLensStore.setState({ activeLensId: 'lens-2' });
    slow.resolve({ rows: [row('e1')], total: 1 });
    await sorting;

    expect(useLensStore.getState().sort).toEqual({ columnKey: 'total', direction: 'asc' });
    expect(useLensStore.getState().rows).toEqual([]);
  });

  it('refetches with the new column filter and ignores an answer overtaken by a newer filter', async () => {
    useLensStore.setState({ activeLensId: 'lens-1', rows: [] });
    const slow = deferred<{ rows: LensRow[]; total: number }>();
    api.getLensRows.mockReturnValueOnce(slow.promise).mockResolvedValueOnce({ rows: [row('e2')], total: 1 });

    const first = useLensStore
      .getState()
      .setColumnFilter('vendor', { key: 'vendor', values: ['A'], includeEmpty: false });
    await useLensStore.getState().setColumnFilter('vendor', { key: 'vendor', values: ['B'], includeEmpty: false });
    slow.resolve({ rows: [row('e1')], total: 1 });
    await first;

    expect(useLensStore.getState().rows.map((r) => r.emailId)).toEqual(['e2']);
    expect(api.getLensRows).toHaveBeenLastCalledWith('lens-1', {
      sort: undefined,
      filters: [{ key: 'vendor', values: ['B'], includeEmpty: false }],
    });
  });

  it('switches between the Lens rows and the excluded rows', async () => {
    useLensStore.setState({ activeLensId: 'lens-1' });

    await useLensStore.getState().setShowExcluded(true);
    expect(useLensStore.getState().rows.map((r) => r.emailId)).toEqual(['x1']);
    expect(useLensStore.getState().isLoadingRows).toBe(false);

    await useLensStore.getState().setShowExcluded(false);
    expect(useLensStore.getState().rows.map((r) => r.emailId)).toEqual(['e1', 'e2']);
  });
});
