import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { PaperclipIcon } from '@/components/common/MailIcons';
import { LIST_COLUMN_WIDTH, type ListColumns, type ListSort, type ListSortKey } from '@/lib/listColumns';

interface ListColumnHeaderProps {
  /** Which optional columns the rows show. */
  columns: ListColumns;
  onColumnsChange: (columns: ListColumns) => void;
  sort: ListSort;
  /** A click on a sortable header. */
  onSort: (key: ListSortKey) => void;
  /** False while the list keeps its own order (a search, the Snoozed view):
   *  the headers are labels only. */
  sortable: boolean;
  /** The Sent view: the party column names the recipients, and is not sorted
   *  by sender. */
  inSentView: boolean;
}

/**
 * The full-width list's column header. Its cells take the widths of the
 * single-line row's cells (`LIST_COLUMN_WIDTH`), with spacers where the row
 * has its checkbox, unread dot, star and avatar, so the labels sit over their
 * columns. The scrollbar gutter is reserved like the list's, so the right-hand
 * columns stay aligned when the list scrolls.
 */
export function ListColumnHeader({
  columns,
  onColumnsChange,
  sort,
  onSort,
  sortable,
  inSentView,
}: ListColumnHeaderProps) {
  const { t } = useTranslation(['inbox']);
  const [pickerOpen, setPickerOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!pickerOpen) return;
    const close = (e: MouseEvent | KeyboardEvent) => {
      if (e instanceof KeyboardEvent ? e.key === 'Escape' : !rootRef.current?.contains(e.target as Node)) {
        setPickerOpen(false);
      }
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', close);
    return () => {
      document.removeEventListener('mousedown', close);
      document.removeEventListener('keydown', close);
    };
  }, [pickerOpen]);

  const header = (key: ListSortKey, label: string, className: string, alignEnd = false) => {
    const justify = alignEnd ? 'justify-end' : '';
    if (!sortable) {
      return <span className={`flex-shrink-0 flex ${justify} truncate ${className}`}>{label}</span>;
    }
    const active = sort.key === key;
    const ariaLabel = active
      ? t(sort.dir === 'asc' ? 'inbox:listColumns.sortedAscending' : 'inbox:listColumns.sortedDescending', {
          column: label,
        })
      : t('inbox:listColumns.sortBy', { column: label });
    return (
      <button
        type="button"
        data-testid={`list-sort-${key}`}
        aria-label={ariaLabel}
        title={ariaLabel}
        onClick={() => onSort(key)}
        className={`flex-shrink-0 flex items-center gap-0.5 ${justify} min-w-0 rounded hover:text-gray-800 ${
          active ? 'font-semibold text-gray-700' : ''
        } ${className}`}
      >
        <span className="truncate">{label}</span>
        {active && <SortArrow ascending={sort.dir === 'asc'} />}
      </button>
    );
  };

  const toggles: { key: keyof ListColumns; label: string }[] = [
    { key: 'attachment', label: t('inbox:listColumns.attachment') },
    { key: 'size', label: t('inbox:listColumns.size') },
  ];

  return (
    <div ref={rootRef} className="relative border-b border-gray-200 bg-white">
      <div className="@container overflow-hidden [scrollbar-gutter:stable] px-4 py-1.5">
        <div className="flex items-center gap-3 min-w-0 text-xs text-gray-500">
          {/* The row's checkbox, unread dot, star and avatar. */}
          <span className="flex-shrink-0 w-4" aria-hidden="true" />
          <span className="flex-shrink-0 w-1.5" aria-hidden="true" />
          <span className="flex-shrink-0 w-5" aria-hidden="true" />
          <span className="flex-shrink-0 w-6" aria-hidden="true" />
          {inSentView ? (
            <span className={`flex-shrink-0 truncate ${LIST_COLUMN_WIDTH.party}`}>
              {t('inbox:listColumns.recipients')}
            </span>
          ) : (
            header('sender', t('inbox:listColumns.sender'), LIST_COLUMN_WIDTH.party)
          )}
          <div className="flex-1 min-w-0 flex">{header('subject', t('inbox:listColumns.subject'), 'max-w-full')}</div>
          {columns.attachment && (
            <span
              role="img"
              data-testid="list-column-attachment"
              aria-label={t('inbox:listColumns.attachment')}
              title={t('inbox:listColumns.attachment')}
              className={`flex-shrink-0 flex justify-center ${LIST_COLUMN_WIDTH.attachment}`}
            >
              <PaperclipIcon className="w-3.5 h-3.5" />
            </span>
          )}
          {columns.size && header('size', t('inbox:listColumns.size'), LIST_COLUMN_WIDTH.size, true)}
          {header('date', t('inbox:listColumns.date'), LIST_COLUMN_WIDTH.date, true)}
          {/* In the row's ⋮ menu column. */}
          <button
            type="button"
            onClick={() => setPickerOpen((open) => !open)}
            aria-label={t('inbox:listColumns.columns')}
            title={t('inbox:listColumns.columns')}
            aria-expanded={pickerOpen}
            className="flex-shrink-0 w-6 h-6 flex items-center justify-center rounded text-gray-400 hover:text-gray-600 hover:bg-gray-100"
          >
            <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 4v16M15 4v16M4 4h16v16H4z" />
            </svg>
          </button>
        </div>
      </div>
      {pickerOpen && (
        <div className="absolute right-4 top-full mt-1 z-20 min-w-40 bg-white rounded-lg shadow-lg border border-gray-200 py-1">
          {toggles.map(({ key, label }) => (
            <label
              key={key}
              className="flex items-center gap-2 px-3 py-1.5 text-sm text-gray-700 hover:bg-gray-50 cursor-pointer whitespace-nowrap"
            >
              <input
                type="checkbox"
                className="rounded border-gray-300"
                checked={columns[key]}
                onChange={(e) => onColumnsChange({ ...columns, [key]: e.target.checked })}
              />
              {label}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

function SortArrow({ ascending }: { ascending: boolean }) {
  return (
    <svg className="w-3 h-3 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2.5}
        d={ascending ? 'M5 15l7-7 7 7' : 'M19 9l-7 7-7-7'}
      />
    </svg>
  );
}
