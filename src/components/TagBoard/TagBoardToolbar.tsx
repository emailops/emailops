import { useTranslation } from 'react-i18next';
import {
  type CustomRange,
  customRangeOnEdit,
  TAG_BOARD_RANGES,
  TAG_BOARD_TYPES,
  type TagBoardDensity,
  type TagBoardRange,
  type TagBoardType,
} from '@/lib/tagBoard';
import type { EmailCategory } from '@/types';

interface TagBoardToolbarProps {
  tagType: TagBoardType;
  onChangeTagType: (t: TagBoardType) => void;
  range: TagBoardRange;
  onChangeRange: (r: TagBoardRange) => void;
  custom: CustomRange;
  /** Every edit, as typed — see `customRangeOnEdit`. */
  onChangeCustom: (c: CustomRange) => void;
  /** A box was left: put the pair in order. */
  onCommitCustom: () => void;
  /** Gmail categories this scope can offer. Empty hides the row entirely
   *  (IMAP accounts have no categories to filter by). */
  availableCategories: EmailCategory[];
  selectedCategories: Set<EmailCategory>;
  onSelectCategories: (next: Set<EmailCategory>) => void;
  hiddenCount: number;
  onRestoreHidden: () => void;
  /** Bound to the same `junk_flagged_action` preference as the inbox's
   *  checkbox, so the two controls can never disagree. */
  hideJunk: boolean;
  onChangeHideJunk: (hide: boolean) => void;
  density: TagBoardDensity;
  onChangeDensity: (d: TagBoardDensity) => void;
  /** Live search over tag values — reaches blocks ranking never surfaces. */
  search: string;
  onChangeSearch: (value: string) => void;
  isRefreshing: boolean;
  onRefresh: () => void;
  onNewChat?: () => void;
}

/** Segmented control shared by the group-by and range rows. */
function Segmented<T extends string>({
  options,
  value,
  onChange,
  label,
}: {
  options: readonly { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="inline-flex overflow-hidden rounded-md border border-gray-200 text-sm"
    >
      {options.map((o, i) => (
        <button
          key={o.value}
          type="button"
          aria-pressed={o.value === value}
          onClick={() => onChange(o.value)}
          className={`px-3 py-1.5 ${i > 0 ? 'border-l border-gray-200' : ''} ${
            o.value === value ? 'bg-gray-100 font-medium text-gray-900' : 'text-gray-600 hover:bg-gray-50'
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function TagBoardToolbar({
  tagType,
  onChangeTagType,
  range,
  onChangeRange,
  custom,
  onChangeCustom,
  onCommitCustom,
  availableCategories,
  selectedCategories,
  onSelectCategories,
  hiddenCount,
  onRestoreHidden,
  hideJunk,
  onChangeHideJunk,
  density,
  onChangeDensity,
  search,
  onChangeSearch,
  isRefreshing,
  onRefresh,
  onNewChat,
}: TagBoardToolbarProps) {
  const { t } = useTranslation(['tagboard', 'inbox', 'chat']);

  // "All" when everything (or nothing) is picked; otherwise the single choice.
  const activeCategory: 'all' | EmailCategory =
    selectedCategories.size === 0 || selectedCategories.size === availableCategories.length
      ? 'all'
      : ((Array.from(selectedCategories)[0] ?? 'all') as EmailCategory);

  return (
    <header className="flex-shrink-0 border-b border-gray-200 px-6 py-3">
      {/* With the reading pane open the board is about half its width, and a
          non-wrapping row squeezed the subtitle into four lines while the
          controls ran off the right edge. The title takes a zero flex basis so
          it never forces the controls onto a second line at full width; once
          the controls really do not fit they drop below the title and wrap
          within their own rows. */}
      <div className="flex flex-wrap items-start gap-x-4 gap-y-2">
        <div className="min-w-0 flex-1 basis-0">
          <h1 className="truncate text-xl font-semibold text-gray-900">{t('tagboard:title')}</h1>
          <p className="mt-0.5 truncate text-xs text-gray-500">{t('tagboard:subtitle')}</p>
        </div>

        {/* `min-w-0` + `overflow-x-auto` on each row: when the board is the
            narrow column of a three-pane layout (chat docked + thread open) the
            segmented controls cannot shrink, and `items-end` used to push the
            overflow off the left edge, under the sidebar. */}
        <div className="flex min-w-0 max-w-full flex-col items-end gap-2">
          <div className="flex max-w-full flex-wrap items-center justify-end gap-2 overflow-x-auto">
            <span className="text-xs text-gray-500">{t('tagboard:groupBy')}</span>
            <Segmented
              label={t('tagboard:groupBy')}
              value={tagType}
              onChange={onChangeTagType}
              options={TAG_BOARD_TYPES.map((v) => ({ value: v, label: t(`tagboard:types.${v}`) }))}
            />
            <button
              type="button"
              onClick={onRefresh}
              disabled={isRefreshing}
              title={t('tagboard:refresh')}
              className="rounded p-1.5 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600 disabled:opacity-50"
            >
              <svg
                className={`h-4 w-4 ${isRefreshing ? 'animate-spin' : ''}`}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
                />
              </svg>
            </button>
            {onNewChat && (
              <button
                type="button"
                onClick={onNewChat}
                title={t('chat:panel.newChat')}
                aria-label={t('chat:panel.newChat')}
                className="rounded p-1.5 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600"
              >
                <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                  />
                </svg>
              </button>
            )}
          </div>

          <div className="flex max-w-full flex-wrap items-center justify-end gap-2 overflow-x-auto">
            <div className="relative">
              <svg
                aria-hidden="true"
                className="pointer-events-none absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
                />
              </svg>
              <input
                id="tagboard-search"
                type="search"
                value={search}
                onChange={(e) => onChangeSearch(e.target.value)}
                placeholder={t('tagboard:searchPlaceholder')}
                aria-label={t('tagboard:searchPlaceholder')}
                className="w-44 rounded-md border border-gray-200 py-1.5 pl-7 pr-2 text-sm outline-none focus:border-primary-400 focus:ring-2 focus:ring-primary-100"
              />
            </div>

            {/* Block width. `extended` doubles each block, halving the columns. */}
            <div
              role="group"
              aria-label={t('tagboard:density.label')}
              className="inline-flex overflow-hidden rounded-md border border-gray-200"
            >
              <button
                type="button"
                aria-pressed={density === 'granular'}
                onClick={() => onChangeDensity('granular')}
                title={t('tagboard:density.granular')}
                aria-label={t('tagboard:density.granular')}
                className={`px-2 py-1.5 ${
                  density === 'granular' ? 'bg-gray-100 text-gray-900' : 'text-gray-500 hover:bg-gray-50'
                }`}
              >
                <svg className="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
                  <rect x="2" y="3" width="3.4" height="14" rx="1" />
                  <rect x="7.3" y="3" width="3.4" height="14" rx="1" />
                  <rect x="12.6" y="3" width="3.4" height="14" rx="1" />
                </svg>
              </button>
              <button
                type="button"
                aria-pressed={density === 'extended'}
                onClick={() => onChangeDensity('extended')}
                title={t('tagboard:density.extended')}
                aria-label={t('tagboard:density.extended')}
                className={`border-l border-gray-200 px-2 py-1.5 ${
                  density === 'extended' ? 'bg-gray-100 text-gray-900' : 'text-gray-500 hover:bg-gray-50'
                }`}
              >
                <svg className="h-4 w-4" viewBox="0 0 20 20" fill="currentColor" aria-hidden="true">
                  <rect x="2" y="3" width="7.2" height="14" rx="1" />
                  <rect x="10.8" y="3" width="7.2" height="14" rx="1" />
                </svg>
              </button>
            </div>

            <span className="text-xs text-gray-500">{t('tagboard:range.label')}</span>
            <Segmented
              label={t('tagboard:range.label')}
              value={range}
              onChange={onChangeRange}
              options={TAG_BOARD_RANGES.map((v) => ({ value: v, label: t(`tagboard:range.${v}`) }))}
            />
          </div>

          {/* Under the Range control, so the dates read as part of "Custom".
              WebKit anchors the calendar pop-up to the box's LEFT edge and
              does not pull it back inside the window, so with the boxes
              flush right the "to" calendar was cut off on a maximised window.
              The boxes are wider than the pop-up (~140px) for that reason:
              a pop-up that starts inside the box ends inside it too. */}
          {range === 'custom' && (
            <div className="flex items-center gap-2 text-xs text-gray-600">
              <label htmlFor="tagboard-from">{t('tagboard:range.from')}</label>
              <input
                id="tagboard-from"
                type="date"
                value={custom.from}
                // Bounding each field by the other makes a reversed range
                // impossible to enter, rather than silently corrected after.
                max={custom.to || undefined}
                onChange={(e) => onChangeCustom(customRangeOnEdit(custom, 'from', e.target.value))}
                onBlur={onCommitCustom}
                aria-label={t('tagboard:range.from')}
                className="w-40 rounded border border-gray-300 px-2 py-1"
              />
              <label htmlFor="tagboard-to">{t('tagboard:range.to')}</label>
              <input
                id="tagboard-to"
                type="date"
                value={custom.to}
                min={custom.from || undefined}
                onChange={(e) => onChangeCustom(customRangeOnEdit(custom, 'to', e.target.value))}
                onBlur={onCommitCustom}
                aria-label={t('tagboard:range.to')}
                className="w-40 rounded border border-gray-300 px-2 py-1"
              />
            </div>
          )}
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2">
        {availableCategories.length > 0 && (
          <nav className="flex items-center gap-1" aria-label={t('inbox:categoriesAria')}>
            <button
              type="button"
              aria-pressed={activeCategory === 'all'}
              onClick={() => onSelectCategories(new Set(availableCategories))}
              className={`rounded-full px-2.5 py-1 text-xs transition-colors ${
                activeCategory === 'all' ? 'bg-gray-900 text-white' : 'bg-gray-100 text-gray-600 hover:bg-gray-200'
              }`}
            >
              {t('inbox:allCategories')}
            </button>
            {availableCategories.map((c) => (
              <button
                key={c}
                type="button"
                aria-pressed={activeCategory === c}
                onClick={() => onSelectCategories(new Set([c]))}
                className={`rounded-full px-2.5 py-1 text-xs transition-colors ${
                  activeCategory === c ? 'bg-gray-900 text-white' : 'bg-gray-100 text-gray-600 hover:bg-gray-200'
                }`}
              >
                {t(`inbox:categoryFilter.${c}`)}
              </button>
            ))}
          </nav>
        )}

        <label className="flex cursor-pointer select-none items-center gap-2 text-xs text-gray-600">
          <input
            id="tagboard-hide-junk"
            type="checkbox"
            className="rounded border-gray-300"
            checked={hideJunk}
            onChange={(e) => onChangeHideJunk(e.target.checked)}
          />
          <span>{t('inbox:junk.hideFlagged')}</span>
        </label>

        {hiddenCount > 0 && (
          <button
            type="button"
            onClick={onRestoreHidden}
            className="ml-auto text-xs text-primary-600 hover:text-primary-700 hover:underline"
          >
            {t('tagboard:restoreHidden', { count: hiddenCount })}
          </button>
        )}
      </div>
    </header>
  );
}
