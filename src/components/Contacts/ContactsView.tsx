import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { CompanyContactsGroup, Contact, ContactDetail, ContactKind, ContactSort, ContactsPage } from '@/types';
import { CompanyView } from './CompanyView';
import { ContactDetailPanel } from './ContactDetailPanel';
import { ContactRow, SkeletonRows } from './ContactRow';

interface ContactsViewProps {
  accountId: string | null;
  /** Open the compose modal pre-filled with this address in the To field. */
  onComposeTo: (address: string) => void;
  /** Switch to inbox and apply a sender filter for this address. */
  onViewEmailsFrom: (address: string) => void;
}

type ViewMode = 'list' | 'company';

const PAGE_SIZE = 100;

const SORT_OPTIONS: {
  value: ContactSort;
  labelKey: 'mostRecent' | 'relationship' | 'mostEmails' | 'mostReceived' | 'mostSent' | 'nameAZ';
}[] = [
  { value: 'last', labelKey: 'mostRecent' },
  { value: 'score', labelKey: 'relationship' },
  { value: 'total', labelKey: 'mostEmails' },
  { value: 'received', labelKey: 'mostReceived' },
  { value: 'sent', labelKey: 'mostSent' },
  { value: 'name', labelKey: 'nameAZ' },
];

const KIND_CHIPS: { value: 'all' | ContactKind; labelKey: 'all' | 'people' | 'automated' }[] = [
  { value: 'all', labelKey: 'all' },
  { value: 'person', labelKey: 'people' },
  { value: 'automated', labelKey: 'automated' },
];

export function ContactsView({ accountId, onComposeTo, onViewEmailsFrom }: ContactsViewProps) {
  const { t } = useTranslation(['contacts', 'common']);
  const fmt = useFormatters();
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [sort, setSort] = useState<ContactSort>('last');
  const [kindFilter, setKindFilter] = useState<'all' | ContactKind>('all');
  const [companyFilter, setCompanyFilter] = useState<string | null>(null);

  const [page, setPage] = useState<ContactsPage | null>(null);
  const [items, setItems] = useState<Contact[]>([]);
  const [companyGroups, setCompanyGroups] = useState<CompanyContactsGroup[] | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [selectedAddress, setSelectedAddress] = useState<string | null>(null);
  const [detail, setDetail] = useState<ContactDetail | null>(null);
  const [isLoadingDetail, setIsLoadingDetail] = useState(false);

  const fetchIdRef = useRef(0);

  // Debounce search input
  useEffect(() => {
    const t = setTimeout(() => setDebouncedSearch(search.trim()), 200);
    return () => clearTimeout(t);
  }, [search]);

  // Reset selection when account changes
  useEffect(() => {
    setSelectedAddress(null);
    setDetail(null);
    setItems([]);
    setPage(null);
    setCompanyGroups(null);
    setCompanyFilter(null);
  }, [accountId]);

  // Fetch list / company groups whenever filters change
  useEffect(() => {
    if (!accountId) {
      setItems([]);
      setPage(null);
      setCompanyGroups(null);
      return;
    }
    const reqId = ++fetchIdRef.current;
    setIsLoading(true);
    setError(null);

    if (viewMode === 'company') {
      api
        .listContactsByCompany(accountId)
        .then((groups) => {
          if (fetchIdRef.current !== reqId) return;
          setCompanyGroups(groups);
        })
        .catch((e) => {
          if (fetchIdRef.current !== reqId) return;
          setError(errorText(e));
        })
        .finally(() => {
          if (fetchIdRef.current !== reqId) return;
          setIsLoading(false);
        });
      return;
    }

    api
      .listContacts(accountId, {
        search: debouncedSearch || undefined,
        kind: kindFilter === 'all' ? undefined : kindFilter,
        company: companyFilter ?? undefined,
        sort,
        offset: 0,
        limit: PAGE_SIZE,
      })
      .then((p) => {
        if (fetchIdRef.current !== reqId) return;
        setPage(p);
        setItems(p.items);
      })
      .catch((e) => {
        if (fetchIdRef.current !== reqId) return;
        setError(errorText(e));
      })
      .finally(() => {
        if (fetchIdRef.current !== reqId) return;
        setIsLoading(false);
      });
  }, [accountId, viewMode, debouncedSearch, kindFilter, companyFilter, sort]);

  // Load detail when selection changes
  useEffect(() => {
    if (!accountId || !selectedAddress) {
      setDetail(null);
      return;
    }
    const reqId = ++fetchIdRef.current;
    setIsLoadingDetail(true);
    api
      .getContactDetail(accountId, selectedAddress)
      .then((d) => {
        if (fetchIdRef.current !== reqId) return;
        setDetail(d);
      })
      .catch(() => {
        if (fetchIdRef.current !== reqId) return;
        setDetail(null);
      })
      .finally(() => {
        if (fetchIdRef.current !== reqId) return;
        setIsLoadingDetail(false);
      });
  }, [accountId, selectedAddress]);

  const loadMore = useCallback(async () => {
    if (!accountId || !page?.hasMore || isLoadingMore) return;
    setIsLoadingMore(true);
    try {
      const next = await api.listContacts(accountId, {
        search: debouncedSearch || undefined,
        kind: kindFilter === 'all' ? undefined : kindFilter,
        company: companyFilter ?? undefined,
        sort,
        offset: items.length,
        limit: PAGE_SIZE,
      });
      setItems((prev) => [...prev, ...next.items]);
      setPage(next);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsLoadingMore(false);
    }
  }, [accountId, page, isLoadingMore, debouncedSearch, kindFilter, companyFilter, sort, items.length]);

  // Distinct company values for the company filter dropdown (list-mode only,
  // derived from current page so it reflects what the user is looking at).
  const distinctCompanies = useMemo(() => {
    const set = new Set<string>();
    for (const c of items) {
      if (c.company) set.add(c.company);
    }
    return Array.from(set).sort();
  }, [items]);

  if (!accountId) {
    return (
      <div className="flex items-center justify-center flex-1 text-sm text-gray-500 bg-white dark:text-gray-400 dark:bg-surface">
        {t('contacts:view.selectAccount')}
      </div>
    );
  }

  return (
    <div className="flex flex-1 overflow-hidden bg-white dark:bg-surface">
      <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
        {/* Header / toolbar */}
        <div className="px-6 py-4 border-b border-gray-200 flex-shrink-0 space-y-3 dark:border-gray-700">
          <div className="flex items-center justify-between">
            <h1 className="text-xl font-semibold text-gray-900 dark:text-gray-100">{t('contacts:title')}</h1>
            <div className="inline-flex rounded-md border border-gray-200 overflow-hidden text-sm dark:border-gray-700">
              <button
                type="button"
                className={`px-3 py-1.5 ${
                  viewMode === 'list'
                    ? 'bg-gray-100 text-gray-900 font-medium dark:bg-surface-hover dark:text-gray-100'
                    : 'text-gray-600 hover:bg-gray-50 dark:text-gray-400 dark:hover:bg-surface-raised'
                }`}
                onClick={() => setViewMode('list')}
              >
                {t('contacts:view.modeList')}
              </button>
              <button
                type="button"
                className={`px-3 py-1.5 border-l border-gray-200 dark:border-gray-700 ${
                  viewMode === 'company'
                    ? 'bg-gray-100 text-gray-900 font-medium dark:bg-surface-hover dark:text-gray-100'
                    : 'text-gray-600 hover:bg-gray-50 dark:text-gray-400 dark:hover:bg-surface-raised'
                }`}
                onClick={() => setViewMode('company')}
              >
                {t('contacts:view.modeCompany')}
              </button>
            </div>
          </div>

          {viewMode === 'list' && (
            <>
              <div className="flex items-center gap-2">
                <input
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder={t('contacts:view.searchPlaceholder')}
                  className="flex-1 max-w-md px-3 py-2 text-sm border border-gray-300 rounded-lg focus:border-primary-500 focus:ring-2 focus:ring-primary-100 outline-none dark:border-gray-600"
                />
                <Select
                  value={sort}
                  onChange={setSort}
                  options={SORT_OPTIONS.map((o) => ({
                    value: o.value,
                    label: t('contacts:view.sortPrefix', { label: t(`contacts:sort.${o.labelKey}` as const) }),
                  }))}
                  ariaLabel="Sort contacts"
                  variant="light"
                />
                {distinctCompanies.length > 0 && (
                  <Select
                    value={companyFilter ?? ''}
                    onChange={(value) => setCompanyFilter(value || null)}
                    options={[
                      { value: '', label: t('contacts:view.allCompanies') },
                      ...distinctCompanies.map((c) => ({ value: c, label: c })),
                    ]}
                    ariaLabel="Filter by company"
                    variant="light"
                  />
                )}
              </div>

              <div className="flex items-center gap-1">
                {KIND_CHIPS.map((chip) => (
                  <button
                    key={chip.value}
                    type="button"
                    onClick={() => setKindFilter(chip.value)}
                    className={`px-3 py-1 text-xs rounded-full border transition-colors ${
                      kindFilter === chip.value
                        ? 'bg-primary-50 border-primary-300 text-primary-700 dark:bg-primary-900/20 dark:text-primary-300'
                        : 'bg-white border-gray-200 text-gray-600 hover:bg-gray-50 dark:bg-surface dark:border-gray-700 dark:text-gray-400 dark:hover:bg-surface-raised'
                    }`}
                  >
                    {t(`contacts:kind.${chip.labelKey}` as const)}
                  </button>
                ))}
                {companyFilter && (
                  <button
                    type="button"
                    onClick={() => setCompanyFilter(null)}
                    className="px-3 py-1 text-xs rounded-full bg-gray-100 text-gray-700 hover:bg-gray-200 ml-2 dark:bg-surface-hover dark:text-gray-300 dark:hover:bg-gray-700"
                  >
                    {t('contacts:view.companyChip', { name: companyFilter })}
                  </button>
                )}
              </div>
            </>
          )}
        </div>

        {/* Body */}
        <div className="flex-1 overflow-y-auto">
          {error && (
            <div className="px-6 py-3 text-sm text-rose-600 bg-rose-50 border-b border-rose-100 dark:text-rose-400 dark:bg-rose-900/20 dark:border-rose-900/40">
              {error}
            </div>
          )}

          {isLoading && items.length === 0 && companyGroups === null ? (
            <SkeletonRows />
          ) : viewMode === 'company' ? (
            <CompanyView
              groups={companyGroups ?? []}
              selectedAddress={selectedAddress}
              onSelect={setSelectedAddress}
              onFilterToCompany={(co) => {
                setViewMode('list');
                setCompanyFilter(co);
              }}
            />
          ) : items.length === 0 ? (
            <div className="flex items-center justify-center h-full text-sm text-gray-500 dark:text-gray-400">
              {debouncedSearch || kindFilter !== 'all' || companyFilter
                ? t('contacts:view.noMatch')
                : t('contacts:view.noContactsYet')}
            </div>
          ) : (
            <>
              <div className="divide-y divide-gray-100 dark:divide-gray-800">
                {items.map((c) => (
                  <ContactRow
                    key={c.email}
                    contact={c}
                    selected={c.email === selectedAddress}
                    onClick={() => setSelectedAddress(c.email)}
                  />
                ))}
              </div>
              <div className="px-6 py-4 flex items-center justify-between text-xs text-gray-500 border-t border-gray-100 dark:text-gray-400 dark:border-gray-800">
                <span>
                  {t(
                    (page?.total ?? items.length) === 1
                      ? 'contacts:view.countOfTotalOne'
                      : 'contacts:view.countOfTotalOther',
                    {
                      shown: fmt.number(items.length),
                      total: fmt.number(page?.total ?? items.length),
                    },
                  )}
                </span>
                {page?.hasMore && (
                  <button
                    type="button"
                    onClick={loadMore}
                    disabled={isLoadingMore}
                    className="px-3 py-1 text-xs border border-gray-300 rounded hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:hover:bg-surface-raised"
                  >
                    {isLoadingMore ? t('contacts:view.loading') : t('contacts:view.loadMore')}
                  </button>
                )}
              </div>
            </>
          )}
        </div>
      </div>

      {selectedAddress &&
        (isLoadingDetail && !detail ? (
          <div className="w-96 flex-shrink-0 border-l border-gray-200 bg-white flex items-center justify-center dark:border-gray-700 dark:bg-surface">
            <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-primary-600" />
          </div>
        ) : detail ? (
          <ContactDetailPanel
            detail={detail}
            onClose={() => setSelectedAddress(null)}
            onComposeTo={onComposeTo}
            onViewEmailsFrom={onViewEmailsFrom}
          />
        ) : (
          <div className="w-96 flex-shrink-0 border-l border-gray-200 bg-white flex items-center justify-center text-sm text-gray-500 p-6 text-center dark:border-gray-700 dark:bg-surface dark:text-gray-400">
            {t('contacts:view.couldNotLoad')}
          </div>
        ))}
    </div>
  );
}
