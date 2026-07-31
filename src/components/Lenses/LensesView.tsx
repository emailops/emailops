// Lenses view — table of AI-extracted rows for the currently selected Lens.
// Sidebar lists Lenses; this view shows the active one. Cells render based on
// the per-column type (currency, date, boolean, enum, etc.).

import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Select } from '@/components/shared/Select';
import { errorText } from '@/lib/errors';
import { useLensStore } from '@/stores/lensStore';
import { useLogStore } from '@/stores/logStore';
import type { LensColumn, LensRow } from '@/types';

import { LensConfigModal } from './LensConfigModal';
import { LensCreateModal } from './LensCreateModal';
import { LensRowDrawer } from './LensRowDrawer';
import { LensRunHistoryDialog } from './LensRunHistoryDialog';
import { EmptyState, LensTable, NoRowsState } from './LensTable';

interface LensesViewProps {
  /** When non-null, focus this Lens on mount. */
  initialLensId?: string | null;
}

export function LensesView({ initialLensId }: LensesViewProps) {
  const { t } = useTranslation(['common', 'lenses']);
  const {
    lenses,
    activeLensId,
    activeLens,
    rows,
    totalRows,
    isLoadingRows,
    sort,
    error,
    runStatus,
    initialize,
    selectLens,
    setSort,
    deleteLens,
    updateLens,
    runLens,
    cancelRun,
    startStatusListener,
  } = useLensStore();

  const [showCreate, setShowCreate] = useState(false);
  const [drawerRow, setDrawerRow] = useState<LensRow | null>(null);
  // "quarter" is a synthetic group derived from the email timestamp; the rest
  // are schema column keys. Default to quarter so the table opens with a
  // useful chronological breakdown.
  const [groupBy, setGroupBy] = useState<string | null>('quarter');

  // Columns that make sense to group by (enum or string types).
  const groupableColumns = useMemo(
    () => (activeLens?.schema.columns ?? []).filter((c) => c.type === 'enum' || c.type === 'string'),
    [activeLens],
  );

  // Reset group-by to the quarter default when the active lens changes —
  // schema column keys differ between lenses but "quarter" is always valid.
  useEffect(() => {
    setGroupBy('quarter');
  }, [activeLensId]);

  useEffect(() => {
    void initialize();
  }, [initialize]);

  // Auto-select the first Lens (or the requested one) once the list loads.
  useEffect(() => {
    if (activeLensId || lenses.length === 0) return;
    const target = initialLensId && lenses.some((l) => l.id === initialLensId) ? initialLensId : lenses[0]?.id;
    if (target) void selectLens(target);
  }, [activeLensId, lenses, initialLensId, selectLens]);

  // Subscribe to backend run events for live progress badges.
  useEffect(() => {
    let unlisten: undefined | (() => void);
    void startStatusListener().then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [startStatusListener]);

  const status = activeLensId ? runStatus[activeLensId] : null;
  const isRunning = status?.state === 'running';
  const [showHistory, setShowHistory] = useState(false);
  const [showConfig, setShowConfig] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Auto-cancel a pending delete confirmation if the user switches Lenses or
  // doesn't follow through within a few seconds.
  useEffect(() => {
    if (!confirmDelete) return;
    const t = setTimeout(() => setConfirmDelete(false), 4000);
    return () => clearTimeout(t);
  }, [confirmDelete, activeLensId]);

  useEffect(() => {
    setConfirmDelete(false);
  }, [activeLensId]);

  // Inline rename state — click the header title to enter edit mode.
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const renameInputRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (renaming) renameInputRef.current?.focus();
  }, [renaming]);
  const commitRename = async () => {
    if (!activeLens) return;
    const next = renameValue.trim();
    setRenaming(false);
    if (!next || next === activeLens.name) return;
    try {
      await updateLens(activeLens.id, { name: next });
      useLogStore.getState().addLog('success', 'system', `Renamed Lens to "${next}"`);
    } catch (e) {
      useLogStore.getState().addLog('error', 'system', `Rename failed: ${errorText(e)}`);
    }
  };

  return (
    <div className="flex h-full flex-1 flex-col overflow-hidden bg-[#1e1e1e] text-gray-200">
      {/* Header */}
      <div className="flex items-center justify-between gap-3 border-b border-gray-700 px-5 py-3">
        <div className="min-w-0 flex-1">
          {activeLens && renaming ? (
            <input
              ref={renameInputRef}
              type="text"
              value={renameValue}
              onChange={(e) => setRenameValue(e.currentTarget.value)}
              onBlur={() => void commitRename()}
              onKeyDown={(e) => {
                if (e.key === 'Enter') void commitRename();
                else if (e.key === 'Escape') setRenaming(false);
              }}
              className="w-full max-w-md rounded border border-blue-500 bg-[#1e1e1e] px-2 py-0.5 text-sm font-semibold text-gray-100 focus:outline-none"
            />
          ) : (
            <h1
              className={`truncate text-sm font-semibold text-gray-100 ${activeLens ? 'cursor-text hover:text-white' : ''}`}
              title={activeLens ? 'Click to rename' : undefined}
              onClick={() => {
                if (!activeLens) return;
                setRenameValue(activeLens.name);
                setRenaming(true);
              }}
            >
              {activeLens ? activeLens.name : 'Lenses'}
            </h1>
          )}
          {activeLens && (
            <p className="mt-0.5 truncate text-xs text-gray-400">
              {/* Backend returns total = -1 when COUNT is skipped (infinite scroll); */}
              {/* in that case fall back to the count of rows currently loaded. */}
              {totalRows >= 0 ? totalRows : rows.length} row
              {(totalRows >= 0 ? totalRows : rows.length) === 1 ? '' : 's'}
              {isRunning && (
                <span className="ml-2 text-blue-400">
                  ↻ running{' '}
                  {(status?.total ?? 0) > 0
                    ? `${status?.processed ?? 0}/${status?.total ?? 0} (${Math.min(
                        100,
                        Math.round(((status?.processed ?? 0) / Math.max(1, status?.total ?? 1)) * 100),
                      )}%)`
                    : `(${status?.processed ?? 0} processed)`}
                  {(status?.failed ?? 0) > 0 && <span className="ml-1 text-red-400">· {status?.failed} failed</span>}
                </span>
              )}
              {status?.state === 'error' && (
                <span className="ml-2 text-red-400" title={status.lastError ?? undefined}>
                  last run failed{status.lastError ? `: ${status.lastError}` : ''}
                </span>
              )}
              {!isRunning && status && status.failed > 0 && (
                <span className="ml-2 text-yellow-400">
                  {status.failed} row{status.failed === 1 ? '' : 's'} failed extraction — click &ldquo;Run
                  backfill&rdquo; to retry
                </span>
              )}
            </p>
          )}
        </div>
        <div className="flex items-center gap-2">
          {activeLens && (
            <label className="flex items-center gap-1 text-[11px] text-gray-400">
              {t('lenses:groupBy')}
              <Select
                value={groupBy ?? ''}
                options={[
                  { value: '', label: t('lenses:dashes') },
                  { value: 'quarter', label: t('lenses:quarter') },
                  ...groupableColumns.map((c) => ({ value: c.key, label: c.label })),
                ]}
                onChange={(value) => setGroupBy(value || null)}
                ariaLabel={t('lenses:groupBy')}
                size="xs"
              />
            </label>
          )}
          {activeLens && (
            <>
              {isRunning ? (
                <button
                  type="button"
                  onClick={() => void cancelRun(activeLens.id)}
                  className="rounded border border-yellow-600 px-3 py-1 text-xs text-yellow-300 hover:bg-yellow-900/30"
                >
                  {t('lenses:cancelRun')}
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => void runLens(activeLens.id, 'backfill')}
                  className="rounded border border-gray-600 px-3 py-1 text-xs text-gray-200 hover:bg-gray-700"
                >
                  {t('lenses:runBackfill')}
                </button>
              )}
              <button
                type="button"
                onClick={() => setShowConfig(true)}
                className="rounded border border-gray-600 px-3 py-1 text-xs text-gray-200 hover:bg-gray-700"
                title={t('lenses:configTooltip')}
              >
                Config
              </button>
              <button
                type="button"
                onClick={() => setShowHistory(true)}
                className="rounded border border-gray-600 px-3 py-1 text-xs text-gray-200 hover:bg-gray-700"
                title={t('lenses:historyTooltip')}
              >
                History
              </button>
              {confirmDelete ? (
                <button
                  type="button"
                  onClick={() => {
                    const name = activeLens.name;
                    const id = activeLens.id;
                    setConfirmDelete(false);
                    void deleteLens(id)
                      .then(() => {
                        useLogStore.getState().addLog('success', 'system', `Deleted Lens "${name}"`);
                      })
                      .catch((e) => {
                        useLogStore
                          .getState()
                          .addLog('error', 'system', `Failed to delete Lens "${name}": ${errorText(e)}`);
                      });
                  }}
                  className="rounded border border-red-500 bg-red-900/40 px-3 py-1 text-xs font-medium text-red-200 hover:bg-red-900/60"
                  title={t('lenses:deleteConfirmTooltip')}
                >
                  {t('lenses:deleteConfirm')}
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => setConfirmDelete(true)}
                  className="rounded border border-red-700/60 px-3 py-1 text-xs text-red-300 hover:bg-red-900/40"
                >
                  Delete
                </button>
              )}
            </>
          )}
          <button
            type="button"
            onClick={() => setShowCreate(true)}
            className="rounded bg-blue-600 px-3 py-1 text-xs font-medium text-white hover:bg-blue-500"
          >
            + New Lens
          </button>
        </div>
      </div>

      {isRunning && (status?.total ?? 0) > 0 && (
        <div className="border-b border-gray-800 bg-[#171717] px-5 py-1.5">
          <div className="h-1 w-full overflow-hidden rounded bg-gray-800">
            <div
              className="h-full bg-blue-500 transition-all"
              style={{
                width: `${Math.min(
                  100,
                  Math.round(((status?.processed ?? 0) / Math.max(1, status?.total ?? 1)) * 100),
                )}%`,
              }}
            />
          </div>
        </div>
      )}

      {error && <div className="border-b border-red-700/50 bg-red-900/30 px-5 py-2 text-xs text-red-300">{error}</div>}

      {/* Body */}
      <div className="min-h-0 flex-1 overflow-auto">
        {!activeLens ? (
          <EmptyState onCreate={() => setShowCreate(true)} />
        ) : isLoadingRows ? (
          <div className="p-8 text-center text-xs text-gray-500">{t('lenses:loadingRows')}</div>
        ) : rows.length === 0 ? (
          <NoRowsState
            onRun={() => void runLens(activeLens.id, 'backfill')}
            isRunning={isRunning}
            processed={status?.processed ?? 0}
            total={status?.total ?? 0}
          />
        ) : (
          <LensTable
            columns={activeLens.schema.columns}
            rows={rows}
            sort={sort}
            onSortChange={(s) => void setSort(s)}
            onOverride={(emailId, key, value) =>
              void useLensStore.getState().updateRowOverride(emailId, { [key]: value })
            }
            onReextract={(emailId) => void useLensStore.getState().reextractRow(emailId)}
            onExclude={(emailId) => void useLensStore.getState().excludeRow(emailId)}
            onOpenRow={setDrawerRow}
            groupBy={groupBy}
          />
        )}
      </div>

      <LensRowDrawer row={drawerRow} onClose={() => setDrawerRow(null)} />

      <LensRunHistoryDialog
        lensId={activeLensId}
        lensName={activeLens?.name ?? ''}
        open={showHistory}
        onClose={() => setShowHistory(false)}
      />

      <LensConfigModal lens={activeLens} open={showConfig} onClose={() => setShowConfig(false)} />

      <LensCreateModal
        open={showCreate}
        onClose={() => setShowCreate(false)}
        onCreated={(lens) => {
          setShowCreate(false);
          void selectLens(lens.id);
        }}
      />
    </div>
  );
}

// Memo helper so consumers can use the columns without re-rendering on
// every store tick — exported for tests/storybook ergonomics.
export function useLensColumns(): LensColumn[] {
  const lens = useLensStore((s) => s.activeLens);
  return useMemo(() => lens?.schema.columns ?? [], [lens]);
}
