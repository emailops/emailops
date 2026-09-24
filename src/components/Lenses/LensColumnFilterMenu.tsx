// Excel-style filter for one Lens column: a funnel in the header opens every
// value the column holds (across all rows, not just the loaded page) with a
// checkbox each; the ticked set becomes a backend filter on the rows.

import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';

import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { LensColumn, LensColumnFilter, LensColumnValueCount } from '@/types';

import { columnValueLabel, filterFromSelection } from './columnFilter';

/** Above this many values the list gets a search field. */
const SEARCH_THRESHOLD = 8;

interface LensColumnFilterMenuProps {
  lensId: string;
  column: LensColumn;
  active: LensColumnFilter | undefined;
  onApply: (filter: LensColumnFilter | null) => void;
}

function initialSelection(values: LensColumnValueCount[], active: LensColumnFilter | undefined): Set<string | null> {
  if (!active) return new Set(values.map((v) => v.value));
  const picked = new Set<string | null>(active.values);
  if (active.includeEmpty) picked.add(null);
  return picked;
}

export function LensColumnFilterMenu({ lensId, column, active, onApply }: LensColumnFilterMenuProps) {
  const { t } = useTranslation(['lenses', 'common']);
  const [open, setOpen] = useState(false);
  const [values, setValues] = useState<LensColumnValueCount[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string | null>>(new Set());
  const [search, setSearch] = useState('');
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Fetch fresh values on every open: rows change as runs and edits land.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setValues(null);
    setLoadError(null);
    setSearch('');
    api
      .getLensColumnValues(lensId, column.key)
      .then((vs) => {
        if (cancelled) return;
        setValues(vs);
        setSelected(initialSelection(vs, active));
      })
      .catch((e: unknown) => {
        if (!cancelled) setLoadError(errorText(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, lensId, column.key, active]);

  // Escape or a click outside closes without applying.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener('keydown', onKey);
    window.addEventListener('mousedown', onDown);
    return () => {
      window.removeEventListener('keydown', onKey);
      window.removeEventListener('mousedown', onDown);
    };
  }, [open]);

  const labels = {
    empty: t('lenses:table.filter.empty'),
    yes: t('common:actions.yes'),
    no: t('common:actions.no'),
  };
  const label = (v: string | null) => columnValueLabel(v, column.type, labels);
  const needle = search.trim().toLowerCase();
  const visible = (values ?? []).filter((v) => !needle || label(v.value).toLowerCase().includes(needle));

  const toggle = (v: string | null) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(v)) next.delete(v);
      else next.add(v);
      return next;
    });
  const allVisibleTicked = visible.length > 0 && visible.every((v) => selected.has(v.value));
  const toggleAllVisible = () =>
    setSelected((prev) => {
      const next = new Set(prev);
      for (const v of visible) {
        if (allVisibleTicked) next.delete(v.value);
        else next.add(v.value);
      }
      return next;
    });

  const apply = () => {
    onApply(
      filterFromSelection(
        column.key,
        (values ?? []).map((v) => v.value),
        selected,
      ),
    );
    setOpen(false);
  };
  const clear = () => {
    onApply(null);
    setOpen(false);
  };

  return (
    <div ref={rootRef} className="relative inline-block">
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation(); // the header click sorts
          setOpen((o) => !o);
        }}
        title={t('lenses:table.filter.button')}
        aria-label={t('lenses:table.filter.button')}
        aria-pressed={!!active}
        className={`ml-1 rounded p-0.5 align-middle ${
          active ? 'text-blue-400' : 'text-gray-500 opacity-60 hover:opacity-100'
        } hover:bg-gray-700`}
      >
        <svg className="h-3 w-3" fill={active ? 'currentColor' : 'none'} stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M3 5h18l-7 8v6l-4 2v-8L3 5z" />
        </svg>
      </button>

      {open && (
        <div
          onClick={(e) => e.stopPropagation()}
          className="absolute left-0 top-full z-30 mt-1 w-64 cursor-default rounded border border-gray-600 bg-[#252526] p-2 text-left font-normal normal-case text-gray-200 shadow-xl"
        >
          {loadError && <p className="text-[11px] text-red-400">{loadError}</p>}
          {!values && !loadError && <p className="text-[11px] text-gray-400">{t('common:state.loading')}</p>}
          {values && (
            <>
              {values.length > SEARCH_THRESHOLD && (
                <input
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder={t('lenses:table.filter.search')}
                  aria-label={t('lenses:table.filter.search')}
                  className="mb-2 w-full rounded border border-gray-600 bg-[#1e1e1e] px-2 py-1 text-[11px] text-gray-100 focus:border-blue-500 focus:outline-none"
                />
              )}
              {values.length === 0 ? (
                <p className="text-[11px] text-gray-400">{t('lenses:table.filter.noValues')}</p>
              ) : (
                <>
                  <label className="flex items-center gap-2 border-b border-gray-700 pb-1 text-[11px] font-medium">
                    <input type="checkbox" checked={allVisibleTicked} onChange={toggleAllVisible} />
                    {t('lenses:table.filter.selectAll')}
                  </label>
                  <div className="mt-1 max-h-60 space-y-0.5 overflow-y-auto">
                    {visible.map((v) => (
                      <label key={v.value ?? '\u0000empty'} className="flex items-center gap-2 text-[11px]">
                        <input type="checkbox" checked={selected.has(v.value)} onChange={() => toggle(v.value)} />
                        <span className={`min-w-0 flex-1 truncate ${v.value === null ? 'italic text-gray-400' : ''}`}>
                          {label(v.value)}
                        </span>
                        <span className="text-gray-500">({v.count})</span>
                      </label>
                    ))}
                  </div>
                </>
              )}
              <div className="mt-2 flex justify-end gap-2 border-t border-gray-700 pt-2">
                <button
                  type="button"
                  onClick={clear}
                  disabled={!active}
                  className="rounded px-2 py-0.5 text-[11px] text-gray-300 hover:bg-gray-700 disabled:opacity-40"
                >
                  {t('lenses:table.filter.clear')}
                </button>
                <button
                  type="button"
                  onClick={apply}
                  className="rounded bg-blue-600 px-2 py-0.5 text-[11px] font-medium text-white hover:bg-blue-500"
                >
                  {t('lenses:table.filter.apply')}
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
