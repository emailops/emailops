import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ActiveFilter, SmartFilter } from '@/types';

function CollapseChevron({ open }: { open: boolean }) {
  return (
    <svg
      className={`w-3 h-3 transition-transform ${open ? 'rotate-0' : '-rotate-90'}`}
      fill="currentColor"
      viewBox="0 0 20 20"
    >
      <path
        fillRule="evenodd"
        d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
        clipRule="evenodd"
      />
    </svg>
  );
}

interface SmartFiltersProps {
  filters: SmartFilter[];
  activeFilter: ActiveFilter | null;
  isLoading: boolean;
  onToggleFilter: (filter: ActiveFilter) => void;
  onClearFilter: () => void;
  onPinFilter: (filter: ActiveFilter) => void;
  onUnpinFilter: (filter: ActiveFilter) => void;
  onRemoveFilter: (filter: ActiveFilter) => void;
  onRefresh: () => void;
  isPinned: (filter: ActiveFilter) => boolean;
}

export function SmartFilters({
  filters,
  activeFilter,
  isLoading,
  onToggleFilter,
  onClearFilter,
  onPinFilter,
  onUnpinFilter,
  onRemoveFilter,
  onRefresh,
  isPinned,
}: SmartFiltersProps) {
  const { t } = useTranslation(['common', 'sidebar']);
  const [hoveredFilter, setHoveredFilter] = useState<string | null>(null);
  const [isOpen, setIsOpen] = useState(true);

  const header = (
    <div className="flex items-center justify-between mb-2">
      <button
        onClick={() => setIsOpen((v) => !v)}
        className="flex items-center gap-1.5 text-xs font-semibold text-gray-400 uppercase tracking-wider hover:text-gray-300"
      >
        <CollapseChevron open={isOpen} />
        {t('sidebar:smartFilters.title')}
      </button>
      <div className="flex items-center gap-1">
        {activeFilter && (
          <button onClick={onClearFilter} className="text-xs text-gray-400 hover:text-gray-300">
            {t('common:actions.clear')}
          </button>
        )}
        <button
          onClick={onRefresh}
          disabled={isLoading}
          className="p-0.5 text-gray-500 hover:text-gray-300 disabled:opacity-50 transition-colors"
          title={t('sidebar:filterActions.recalculate')}
        >
          <svg
            className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`}
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
      </div>
    </div>
  );

  if (isLoading && filters.length === 0) {
    return (
      <section>
        {header}
        {isOpen && (
          <div className="flex items-center gap-2 px-3 py-2 text-xs text-gray-500">
            <div className="animate-spin rounded-full h-3 w-3 border-b-2 border-gray-400"></div>
            {t('common:state.loading')}
          </div>
        )}
      </section>
    );
  }

  if (filters.length === 0) {
    return (
      <section>
        {header}
        {isOpen && <p className="px-3 text-xs text-gray-500">{t('sidebar:smartFilters.empty')}</p>}
      </section>
    );
  }

  const FILTER_LIMIT = 10;
  const TAG_TYPES = new Set(['company', 'priority', 'intent', 'topic']);
  // Fixed render order so the sidebar shows Companies first, then Priority,
  // Intent, Topic — independent of backend emission order.
  const TAG_ORDER = ['company', 'priority', 'intent', 'topic'];
  const contactFilters = filters.filter((f) => !TAG_TYPES.has(f.type)).slice(0, FILTER_LIMIT);
  const tagFilters = filters.filter((f) => TAG_TYPES.has(f.type));

  // Group tag filters by type
  const tagGroups: Record<string, SmartFilter[]> = {};
  for (const f of tagFilters) {
    if (!tagGroups[f.type]) tagGroups[f.type] = [];
    tagGroups[f.type].push(f);
  }
  const orderedTagTypes = TAG_ORDER.filter((t) => tagGroups[t]?.length);
  const labelForType = (type: string) => (type === 'company' ? t('sidebar:smartFilters.companies') : type);

  const renderFilter = (filter: SmartFilter) => {
    const key = `${filter.type}:${filter.value}`;
    const isActive = activeFilter?.type === filter.type && activeFilter?.value === filter.value;
    const isHovered = hoveredFilter === key;
    const pinned = isPinned(filter);

    return (
      <li
        key={key}
        className="relative"
        onMouseEnter={() => setHoveredFilter(key)}
        onMouseLeave={() => setHoveredFilter(null)}
      >
        <button
          onClick={() => onToggleFilter(filter)}
          className={`w-full text-left px-3 py-1.5 rounded-lg text-sm transition-colors flex items-center gap-2 ${
            isActive ? 'bg-primary-600 text-white' : 'text-gray-300 hover:bg-gray-800'
          }`}
        >
          <FilterIcon type={filter.type} className="w-3.5 h-3.5 flex-shrink-0" />
          <span className="truncate flex-1">{filter.value}</span>
          {isHovered ? (
            <span className="flex items-center gap-0.5 flex-shrink-0">
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  pinned ? onUnpinFilter(filter) : onPinFilter(filter);
                }}
                className={`p-0.5 rounded hover:bg-gray-700 ${isActive ? 'hover:bg-primary-500' : ''}`}
                title={pinned ? 'Unpin' : 'Pin'}
              >
                <PinIcon filled={pinned} className="w-3 h-3" />
              </button>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onRemoveFilter(filter);
                }}
                className={`p-0.5 rounded hover:bg-gray-700 ${isActive ? 'hover:bg-primary-500' : ''}`}
                title={t('sidebar:filterActions.hide')}
              >
                <XIcon className="w-3 h-3" />
              </button>
            </span>
          ) : (
            <span className={`text-xs flex-shrink-0 ${isActive ? 'text-primary-200' : 'text-gray-500'}`}>
              {filter.count}
            </span>
          )}
        </button>
      </li>
    );
  };

  return (
    <section>
      {header}

      {isOpen && (
        <>
          {contactFilters.length > 0 && <ul className="space-y-0.5">{contactFilters.map(renderFilter)}</ul>}

          {orderedTagTypes.map((type) => (
            <div key={type} className="mt-3">
              <h3 className="text-[10px] font-semibold text-gray-500 uppercase tracking-wider px-3 mb-1">
                {labelForType(type)}
              </h3>
              <ul className="space-y-0.5">{tagGroups[type].slice(0, FILTER_LIMIT).map(renderFilter)}</ul>
            </div>
          ))}
        </>
      )}
    </section>
  );
}

function FilterIcon({ type, className }: { type: string; className?: string }) {
  if (type === 'domain') {
    return (
      <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M21 12a9 9 0 01-9 9m9-9a9 9 0 00-9-9m9 9H3m9 9a9 9 0 01-9-9m9 9c1.657 0 3-4.03 3-9s-1.343-9-3-9m0 18c-1.657 0-3-4.03-3-9s1.343-9 3-9m-9 9a9 9 0 019-9"
        />
      </svg>
    );
  }

  if (type === 'company') {
    return (
      <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M19 21V5a2 2 0 00-2-2H7a2 2 0 00-2 2v16m14 0h2m-2 0h-5m-9 0H3m2 0h5M9 7h1m-1 4h1m4-4h1m-1 4h1m-5 10v-5a1 1 0 011-1h2a1 1 0 011 1v5m-4 0h4"
        />
      </svg>
    );
  }

  if (type === 'priority' || type === 'intent' || type === 'topic') {
    return (
      <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M7 7h.01M7 3h5c.512 0 1.024.195 1.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.994 1.994 0 013 12V7a4 4 0 014-4z"
        />
      </svg>
    );
  }

  return (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z"
      />
    </svg>
  );
}

function PinIcon({ filled, className }: { filled: boolean; className?: string }) {
  if (filled) {
    return (
      <svg className={className} viewBox="0 0 16 16" fill="currentColor">
        <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z" />
      </svg>
    );
  }

  return (
    <svg className={className} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.2">
      <path d="M4.146.146A.5.5 0 0 1 4.5 0h7a.5.5 0 0 1 .5.5c0 .68-.342 1.174-.646 1.479-.126.125-.25.224-.354.298v4.431l.078.048c.203.127.476.314.751.555C12.36 7.775 13 8.527 13 9.5a.5.5 0 0 1-.5.5h-4v4.5a.5.5 0 0 1-1 0V10h-4A.5.5 0 0 1 3 9.5c0-.973.64-1.725 1.17-2.189A5.921 5.921 0 0 1 5 6.708V2.277a2.77 2.77 0 0 1-.354-.298C4.342 1.674 4 1.179 4 .5a.5.5 0 0 1 .146-.354z" />
    </svg>
  );
}

function XIcon({ className }: { className?: string }) {
  return (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
    </svg>
  );
}
