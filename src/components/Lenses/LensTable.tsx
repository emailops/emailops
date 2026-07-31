// Lens table — renders rows grouped or flat, with sortable column headers
// and inline-editable cells. Extracted from LensesView.tsx.

import { Fragment, useCallback, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { useFormatters } from '@/hooks/useFormatters';
import type { LensColumn, LensRow, LensSortSpec } from '@/types';

// ── Helpers ──────────────────────────────────────────────────────────────────

// Column keys used by the built-in Lens templates (src-tauri/.../templates.rs).
// Their headers are localized at display time via `lenses:columns.builtin.*`.
// User-defined columns (and built-in columns the user has renamed) fall back
// to the label stored on the Lens.
const BUILTIN_COLUMN_KEYS = [
  'amount',
  'by_when',
  'cadence',
  'cancel_url',
  'client',
  'confidence',
  'confirmation_code',
  'currency',
  'days_overdue',
  'days_silent',
  'destination',
  'due_date',
  'end_date',
  'invoice_number',
  'newsletter',
  'next_renewal',
  'paid',
  'priority_guess',
  'promise',
  'provider',
  'question_summary',
  'received_date',
  'recipient',
  'reference',
  'sender_name',
  'sent_date',
  'service',
  'start_date',
  'status',
  'subject',
  'summary',
  'top_topic',
  'transfer_id',
  'travel_type',
  'vendor',
  'worth_clicking',
] as const;

type BuiltinColumnKey = (typeof BUILTIN_COLUMN_KEYS)[number];

function isBuiltinColumnKey(key: string): key is BuiltinColumnKey {
  return (BUILTIN_COLUMN_KEYS as readonly string[]).includes(key);
}

const EMAIL_DATE_OPTIONS: Intl.DateTimeFormatOptions = {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
};

// "Qn YYYY" label for the quarter containing a Unix timestamp (seconds).
// Returns "(none)" when the timestamp is missing/invalid so empty rows still
// land in a clearly-labeled bucket instead of crashing the group sort.
function quarterLabel(timestampSecs: number): string {
  if (!timestampSecs) return '(none)';
  const d = new Date(timestampSecs * 1000);
  if (Number.isNaN(d.getTime())) return '(none)';
  const q = Math.floor(d.getMonth() / 3) + 1;
  return `Q${q} ${d.getFullYear()}`;
}

// Inverse of quarterLabel — used to sort quarter group headers chronologically.
function parseQuarterLabel(label: string): { quarter: number; year: number } | null {
  const m = /^Q([1-4])\s+(\d{4})$/.exec(label);
  if (!m) return null;
  return { quarter: Number.parseInt(m[1], 10), year: Number.parseInt(m[2], 10) };
}

// ── Cells ────────────────────────────────────────────────────────────────────

interface CellProps {
  column: LensColumn;
  value: unknown;
  hasOverride: boolean;
}

function Cell({ column, value, hasOverride }: CellProps) {
  const fmt = useFormatters();
  if (value === null || value === undefined || value === '') {
    return <span className="text-gray-600">—</span>;
  }
  const cls = hasOverride ? 'text-amber-300' : 'text-gray-200';
  switch (column.type) {
    case 'currency': {
      const v = value as { amount?: number; currency?: string };
      if (typeof v.amount !== 'number') return <span className="text-gray-600">—</span>;
      return (
        <span className={cls}>
          {fmt.number(v.amount, { minimumFractionDigits: 2 })} {v.currency ?? ''}
        </span>
      );
    }
    case 'date':
      return <span className={cls}>{String(value)}</span>;
    case 'boolean':
      return <span className={cls}>{value ? 'yes' : 'no'}</span>;
    case 'number':
      return <span className={cls}>{fmt.number(Number(value))}</span>;
    case 'url': {
      const href = String(value);
      return (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="text-blue-400 hover:underline"
          onClick={(e) => e.stopPropagation()}
        >
          {href}
        </a>
      );
    }
    case 'text':
      return <span className={`${cls} block max-w-[24rem] whitespace-pre-wrap break-words`}>{String(value)}</span>;
    default:
      return <span className={cls}>{String(value)}</span>;
  }
}

// Inline-editable wrapper around a Cell.
interface EditableCellProps {
  column: LensColumn;
  value: unknown;
  hasOverride: boolean;
  onSave: (next: unknown) => void;
}

function EditableCell({ column, value, hasOverride, onSave }: EditableCellProps) {
  const { t } = useTranslation(['lenses']);
  const [editing, setEditing] = useState(false);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Currency uses a custom editor (amount only — currency code preserved).
  if (column.type === 'currency') {
    if (!editing) {
      return (
        <span
          onDoubleClick={() => setEditing(true)}
          className="cursor-text"
          title={t('lenses:table.doubleClickToEdit')}
        >
          <Cell column={column} value={value} hasOverride={hasOverride} />
        </span>
      );
    }
    const cur = (value as { amount?: number; currency?: string } | null) ?? {};
    return (
      <input
        ref={inputRef}
        type="number"
        defaultValue={typeof cur.amount === 'number' ? cur.amount : ''}
        // biome-ignore lint/a11y/noAutofocus: cell editor opens on user action; focus must move to the input immediately for keyboard-driven editing
        autoFocus
        onBlur={(e) => {
          const n = Number.parseFloat(e.currentTarget.value);
          if (!Number.isNaN(n)) {
            onSave({ amount: n, currency: cur.currency ?? 'USD' });
          }
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur();
          if (e.key === 'Escape') {
            setEditing(false);
          }
        }}
        className="w-24 rounded border border-blue-500 bg-[#1e1e1e] px-1 py-0.5 text-gray-100 focus:outline-none"
      />
    );
  }

  if (!editing) {
    return (
      <span onDoubleClick={() => setEditing(true)} className="cursor-text" title={t('lenses:table.doubleClickToEdit')}>
        <Cell column={column} value={value} hasOverride={hasOverride} />
      </span>
    );
  }

  // Generic edit: text input. Enum gets a dropdown, boolean a checkbox.
  if (column.type === 'enum' && column.enumValues) {
    return (
      // Select closes and commits immediately on option click (no native
      // blur-to-save step needed); Escape still exits the editor without
      // saving, matching the other editable-cell types below.
      <div onKeyDown={(e) => e.key === 'Escape' && setEditing(false)}>
        <Select
          value={String(value ?? '')}
          options={[
            { value: '', label: t('lenses:noneOption') },
            ...column.enumValues.map((v) => ({ value: v, label: v })),
          ]}
          onChange={(next) => {
            onSave(next || null);
            setEditing(false);
          }}
          ariaLabel={column.label}
          size="xs"
        />
      </div>
    );
  }

  if (column.type === 'boolean') {
    return (
      <input
        type="checkbox"
        // biome-ignore lint/a11y/noAutofocus: cell editor opens on user action; focus must move to the checkbox immediately for keyboard-driven editing
        autoFocus
        defaultChecked={Boolean(value)}
        onBlur={(e) => {
          onSave(e.currentTarget.checked);
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Escape') setEditing(false);
        }}
      />
    );
  }

  const initial = value === null || value === undefined ? '' : String(value);
  return (
    <input
      ref={inputRef}
      type={column.type === 'number' ? 'number' : 'text'}
      defaultValue={initial}
      // biome-ignore lint/a11y/noAutofocus: cell editor opens on user action; focus must move to the input immediately for keyboard-driven editing
      autoFocus
      onBlur={(e) => {
        const raw = e.currentTarget.value;
        if (raw === '') {
          onSave(null);
        } else if (column.type === 'number') {
          const n = Number.parseFloat(raw);
          if (!Number.isNaN(n)) onSave(n);
        } else {
          onSave(raw);
        }
        setEditing(false);
      }}
      onKeyDown={(e) => {
        if (e.key === 'Enter') (e.currentTarget as HTMLInputElement).blur();
        if (e.key === 'Escape') setEditing(false);
      }}
      className="w-full rounded border border-blue-500 bg-[#1e1e1e] px-1 py-0.5 text-gray-100 focus:outline-none"
    />
  );
}

// ── Table row ────────────────────────────────────────────────────────────────

interface LensTableRowProps {
  row: LensRow;
  columns: LensColumn[];
  onOverride: (emailId: string, columnKey: string, value: unknown) => void;
  onReextract: (emailId: string) => void;
  onExclude: (emailId: string) => void;
  onOpenRow: (row: LensRow) => void;
}

function LensTableRow({ row, columns, onOverride, onReextract, onExclude, onOpenRow }: LensTableRowProps) {
  const { t } = useTranslation(['lenses']);
  const fmt = useFormatters();
  return (
    <tr className="group border-b border-gray-800 hover:bg-gray-800/40">
      <td className="px-3 py-2 align-top whitespace-nowrap text-gray-200">
        {fmt.date(row.emailTimestamp, EMAIL_DATE_OPTIONS)}
      </td>
      <td className="px-3 py-2 align-top">
        <button
          type="button"
          onClick={() => onOpenRow(row)}
          className="block max-w-[18rem] cursor-pointer text-left hover:text-blue-300"
          title={t('lenses:table.openSourceEmail')}
        >
          <div className="truncate text-gray-200" title={row.emailSubject}>
            {row.emailSubject || '(no subject)'}
          </div>
          <div className="truncate text-[11px] text-gray-500" title={row.emailSender}>
            {row.emailSender}
          </div>
        </button>
      </td>
      {columns.map((c) => (
        <td key={c.key} className="px-3 py-2 align-top">
          <EditableCell
            column={c}
            value={row.data?.[c.key]}
            hasOverride={row.hasOverrides}
            onSave={(next) => onOverride(row.emailId, c.key, next)}
          />
        </td>
      ))}
      <td className="px-3 py-2 align-top text-right">
        <div className="flex justify-end gap-1 opacity-0 transition-opacity group-hover:opacity-100">
          <button
            type="button"
            onClick={() => onReextract(row.emailId)}
            className="rounded border border-gray-600 px-1.5 py-0.5 text-[10px] text-gray-300 hover:bg-gray-700"
            title={t('lenses:table.reextractRow')}
          >
            ↻
          </button>
          <button
            type="button"
            onClick={() => onExclude(row.emailId)}
            className="rounded border border-red-700/60 px-1.5 py-0.5 text-[10px] text-red-300 hover:bg-red-900/40"
            title="Exclude this row"
          >
            ✕
          </button>
        </div>
      </td>
    </tr>
  );
}

// ── Table ────────────────────────────────────────────────────────────────────

export interface LensTableProps {
  columns: LensColumn[];
  rows: LensRow[];
  sort: LensSortSpec | null;
  onSortChange: (sort: LensSortSpec | null) => void;
  onOverride: (emailId: string, columnKey: string, value: unknown) => void;
  onReextract: (emailId: string) => void;
  onExclude: (emailId: string) => void;
  onOpenRow: (row: LensRow) => void;
  groupBy?: string | null;
}

export function LensTable({
  columns,
  rows,
  sort,
  onSortChange,
  onOverride,
  onReextract,
  onExclude,
  onOpenRow,
  groupBy,
}: LensTableProps) {
  const { t, i18n } = useTranslation(['lenses']);
  const enT = useMemo(() => i18n.getFixedT('en', 'lenses'), [i18n]);
  // Localize built-in column headers, but only when the stored label still
  // matches the English default — if the user renamed the column, keep theirs.
  const columnHeader = useCallback(
    (c: LensColumn): string => {
      if (isBuiltinColumnKey(c.key) && enT(`columns.builtin.${c.key}`) === c.label) {
        return t(`lenses:columns.builtin.${c.key}`);
      }
      return c.label;
    },
    [t, enT],
  );
  const toggleSort = (key: string) => {
    if (!sort || sort.columnKey !== key) {
      onSortChange({ columnKey: key, direction: 'desc' });
    } else if (sort.direction === 'desc') {
      onSortChange({ columnKey: key, direction: 'asc' });
    } else {
      onSortChange(null);
    }
  };

  // Build groups when `groupBy` is set — preserves the incoming row order
  // within each group. The "(none)" bucket catches rows with missing values.
  // `quarter` is a synthetic group derived from the email timestamp; quarter
  // labels are sorted newest-first so users see the latest period at the top.
  const groups = useMemo(() => {
    if (!groupBy) return null;
    const map = new Map<string, LensRow[]>();
    for (const row of rows) {
      let key: string;
      if (groupBy === 'quarter') {
        key = quarterLabel(row.emailTimestamp);
      } else {
        const raw = row.data?.[groupBy];
        key = raw === null || raw === undefined || raw === '' ? '(none)' : String(raw);
      }
      const bucket = map.get(key);
      if (bucket) bucket.push(row);
      else map.set(key, [row]);
    }
    const entries = Array.from(map.entries());
    if (groupBy === 'quarter') {
      // Quarter labels are "Qn YYYY" or "(none)" for missing dates — sort
      // newest-first by (year, quarter), pushing "(none)" to the bottom.
      entries.sort(([a], [b]) => {
        const pa = parseQuarterLabel(a);
        const pb = parseQuarterLabel(b);
        if (!pa && !pb) return 0;
        if (!pa) return 1;
        if (!pb) return -1;
        if (pa.year !== pb.year) return pb.year - pa.year;
        return pb.quarter - pa.quarter;
      });
    }
    return entries;
  }, [groupBy, rows]);

  const colSpan = columns.length + 3;

  return (
    <table className="w-full text-left text-xs">
      <thead className="sticky top-0 bg-[#252526] text-gray-400">
        <tr>
          <th className="border-b border-gray-700 px-3 py-2 font-medium whitespace-nowrap">{t('lenses:table.date')}</th>
          <th className="border-b border-gray-700 px-3 py-2 font-medium">{t('lenses:table.email')}</th>
          {columns.map((c) => (
            <th
              key={c.key}
              className="cursor-pointer select-none border-b border-gray-700 px-3 py-2 font-medium hover:text-gray-200"
              title={c.description}
              onClick={() => toggleSort(c.key)}
            >
              {columnHeader(c)}
              {sort?.columnKey === c.key && (
                <span className="ml-1 text-[10px] text-gray-500">{sort.direction === 'asc' ? '▲' : '▼'}</span>
              )}
            </th>
          ))}
          <th className="border-b border-gray-700 px-3 py-2" />
        </tr>
      </thead>
      <tbody>
        {groups
          ? groups.map(([label, bucket]) => (
              <Fragment key={label}>
                <tr className="bg-[#252526]">
                  <td
                    colSpan={colSpan}
                    className="border-b border-gray-700 px-3 py-1.5 text-[11px] font-semibold uppercase tracking-wide text-gray-400"
                  >
                    {label} <span className="ml-1 text-gray-500">({bucket.length})</span>
                  </td>
                </tr>
                {bucket.map((row) => (
                  <LensTableRow
                    key={row.emailId}
                    row={row}
                    columns={columns}
                    onOverride={onOverride}
                    onReextract={onReextract}
                    onExclude={onExclude}
                    onOpenRow={onOpenRow}
                  />
                ))}
              </Fragment>
            ))
          : rows.map((row) => (
              <LensTableRow
                key={row.emailId}
                row={row}
                columns={columns}
                onOverride={onOverride}
                onReextract={onReextract}
                onExclude={onExclude}
                onOpenRow={onOpenRow}
              />
            ))}
      </tbody>
    </table>
  );
}

// ── Empty/zero states ────────────────────────────────────────────────────────

export function EmptyState({ onCreate }: { onCreate: () => void }) {
  const { t } = useTranslation(['lenses']);
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <h2 className="text-base font-semibold text-gray-200">{t('lenses:empty')}</h2>
      <p className="max-w-md text-xs text-gray-400">{t('lenses:emptyDescription')}</p>
      <button
        type="button"
        onClick={onCreate}
        className="mt-2 rounded bg-blue-600 px-4 py-1.5 text-xs font-medium text-white hover:bg-blue-500"
      >
        {t('lenses:createLens')}
      </button>
    </div>
  );
}

export function NoRowsState({
  onRun,
  isRunning,
  processed,
  total,
}: {
  onRun: () => void;
  isRunning: boolean;
  processed: number;
  total: number;
}) {
  const { t } = useTranslation(['lenses']);
  const pct = total > 0 ? Math.min(100, Math.round((processed / Math.max(1, total)) * 100)) : 0;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <h2 className="text-base font-semibold text-gray-200">{t('lenses:emptyRows')}</h2>
      <p className="max-w-md text-xs text-gray-400">
        {isRunning && total > 0
          ? `Extracting from ${total} matching email${total === 1 ? '' : 's'} — this can take a while on the first run.`
          : 'Run an extraction over the matching emails to populate this table.'}
      </p>
      {isRunning && total > 0 && (
        <div className="w-72 max-w-full">
          <div className="h-1.5 w-full overflow-hidden rounded bg-gray-800">
            <div className="h-full bg-blue-500 transition-all" style={{ width: `${pct}%` }} />
          </div>
          <p className="mt-1 text-[11px] text-gray-500">
            {processed} of {total} processed ({pct}%)
          </p>
        </div>
      )}
      <button
        type="button"
        onClick={onRun}
        disabled={isRunning}
        className="mt-2 rounded bg-blue-600 px-4 py-1.5 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-50"
      >
        {isRunning ? (total > 0 ? `Running… ${processed}/${total}` : 'Running…') : 'Run backfill'}
      </button>
    </div>
  );
}
