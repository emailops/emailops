// Memory inspector — read/lightly-editable view of durable facts the memory
// subsystem has learned about the user, contacts, domains, and projects.
// Facts are grouped by `subjectKind`. Candidates show promote/retire buttons;
// promoted facts can be edited or retired.

import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EmailPreviewById } from '@/components/shared/EmailPreviewById';
import { getMemoryConfig } from '@/lib/api';
import { useMemoryStore } from '@/stores/memoryStore';
import type { MemoryConfig, MemoryFact } from '@/types';
import { EligibilityBanner } from './EligibilityBanner';
import { StatusGroup } from './FactList';
import { CompanyChip, StatusToggle } from './MemoryFilters';

interface MemoryViewProps {
  accountId: string | null;
}

const STATUS_ORDER = ['promoted', 'candidate', 'retired'] as const;

export function MemoryView({ accountId }: MemoryViewProps) {
  const { t } = useTranslation(['memory', 'common']);
  const {
    facts,
    factCounts,
    isLoadingFacts,
    factStatusFilter,
    error,
    loadFacts,
    setFactStatusFilter,
    refreshFacts,
    refreshFactCounts,
  } = useMemoryStore();

  const [search, setSearch] = useState('');
  const [companyFilter, setCompanyFilter] = useState<string | null>(null);
  const [selectedFactId, setSelectedFactId] = useState<string | null>(null);
  const [memCfg, setMemCfg] = useState<MemoryConfig | null>(null);

  useEffect(() => {
    if (accountId) {
      void loadFacts(accountId);
    }
  }, [accountId, loadFacts]);

  // Load memory config so we can show which emails are eligible for extraction.
  useEffect(() => {
    let cancelled = false;
    getMemoryConfig()
      .then((cfg) => {
        if (!cancelled) setMemCfg(cfg);
      })
      .catch(() => {
        // Non-fatal: banner will simply not render.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Rank companies by count so the most-frequent chip sits on the left.
  const companyChips = useMemo(() => {
    const counts = new Map<string, number>();
    for (const f of facts) {
      if (!f.company) continue;
      counts.set(f.company, (counts.get(f.company) ?? 0) + 1);
    }
    return [...counts.entries()]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([company, count]) => ({ company, count }));
  }, [facts]);

  // If the active company filter disappears from the data (e.g. after refresh)
  // drop it so we don't show "no results" silently.
  useEffect(() => {
    if (companyFilter && !companyChips.some((c) => c.company === companyFilter)) {
      setCompanyFilter(null);
    }
  }, [companyFilter, companyChips]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return facts.filter((f) => {
      if (companyFilter && f.company !== companyFilter) return false;
      if (!needle) return true;
      return (
        f.fact.toLowerCase().includes(needle) ||
        f.subjectKey.toLowerCase().includes(needle) ||
        (f.company ? f.company.toLowerCase().includes(needle) : false)
      );
    });
  }, [facts, search, companyFilter]);

  // Drop the selection if the fact disappears (deleted/retired filter, or
  // switched account).
  useEffect(() => {
    if (selectedFactId && !facts.some((f) => f.id === selectedFactId)) {
      setSelectedFactId(null);
    }
  }, [facts, selectedFactId]);

  const selectedFact = useMemo(
    () => (selectedFactId ? (facts.find((f) => f.id === selectedFactId) ?? null) : null),
    [facts, selectedFactId],
  );

  // Two-level grouping: status (Promoted / Candidate / Retired) → kind. The
  // status toggle above can collapse this to a single status group.
  const groupsByStatus = useMemo(() => {
    const byStatus: Record<string, Record<string, MemoryFact[]>> = {};
    for (const f of filtered) {
      const s = f.status || 'candidate';
      const k = f.subjectKind || 'other';
      byStatus[s] ??= {};
      byStatus[s][k] ??= [];
      byStatus[s][k].push(f);
    }
    for (const kindMap of Object.values(byStatus)) {
      for (const list of Object.values(kindMap)) {
        list.sort((a, b) => b.score - a.score);
      }
    }
    return byStatus;
  }, [filtered]);

  if (!accountId) {
    return (
      <div className="flex flex-col flex-1 items-center justify-center text-sm text-gray-500 bg-white">
        {t('memory:view.selectAccount')}
      </div>
    );
  }

  return (
    <div className="flex flex-1 overflow-hidden bg-white">
      {/* Left: fact list */}
      <div className="flex flex-col w-[560px] flex-shrink-0 border-r border-gray-200 overflow-hidden">
        <div className="px-6 py-4 border-b border-gray-200 flex-shrink-0 flex items-center justify-between">
          <div>
            <h1 className="text-xl font-semibold text-gray-900">{t('memory:title')}</h1>
            <p className="text-xs text-gray-500 mt-0.5">
              {t('memory:view.totalCount', { count: factCounts.total })}
              {factCounts.promoted > 0 && t('memory:view.consolidatedSuffix', { count: factCounts.promoted })}
              {factCounts.candidate > 0 && t('memory:view.candidateSuffix', { count: factCounts.candidate })}
            </p>
          </div>
          <button
            type="button"
            onClick={() => {
              void refreshFacts();
              void refreshFactCounts();
            }}
            className="text-xs text-gray-500 hover:text-gray-700"
          >
            {t('common:actions.refresh')}
          </button>
        </div>

        {error && <div className="px-6 py-2 bg-red-50 border-b border-red-200 text-sm text-red-700">{error}</div>}

        {memCfg && <EligibilityBanner cfg={memCfg} />}

        <div className="px-6 py-3 border-b border-gray-200 flex items-center gap-3 flex-shrink-0">
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder={t('memory:filters.search')}
            className="flex-1 px-3 py-1.5 text-sm border border-gray-300 rounded-md focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
          <StatusToggle value={factStatusFilter} onChange={(v) => void setFactStatusFilter(v)} />
        </div>

        {companyChips.length > 0 && (
          <div className="px-6 py-2 border-b border-gray-100 flex items-center gap-2 flex-wrap flex-shrink-0">
            <span className="text-xs text-gray-500 mr-1">{t('memory:view.companyLabel')}</span>
            <CompanyChip
              label={t('memory:filters.allChip')}
              count={facts.length}
              active={companyFilter === null}
              onClick={() => setCompanyFilter(null)}
            />
            {companyChips.map(({ company, count }) => (
              <CompanyChip
                key={company}
                label={company}
                count={count}
                active={companyFilter === company}
                onClick={() => setCompanyFilter(companyFilter === company ? null : company)}
              />
            ))}
          </div>
        )}

        <div className="flex-1 overflow-y-auto p-6 space-y-8">
          {isLoadingFacts ? (
            <div className="text-sm text-gray-500">{t('memory:view.loadingFacts')}</div>
          ) : filtered.length === 0 ? (
            <div className="text-sm text-gray-500 italic">
              {facts.length === 0 ? t('memory:view.noFactsYet') : t('memory:view.noMatches')}
            </div>
          ) : (
            STATUS_ORDER.map((key) => {
              const kindMap = groupsByStatus[key];
              if (!kindMap) return null;
              const total = Object.values(kindMap).reduce((acc, list) => acc + list.length, 0);
              if (total === 0) return null;
              return (
                <StatusGroup
                  key={key}
                  title={t(`memory:status.${key}.title` as const)}
                  hint={t(`memory:status.${key}.hint` as const)}
                  total={total}
                  kindMap={kindMap}
                  selectedFactId={selectedFactId}
                  onSelectFact={setSelectedFactId}
                />
              );
            })
          )}
        </div>
      </div>

      {/* Right: originating email preview */}
      <div className="flex-1 min-w-0 overflow-hidden">
        <EmailPreviewById
          accountId={accountId}
          emailId={selectedFact?.sourceEmailId ?? null}
          hasSelection={selectedFact !== null}
          emptyMessage={t('memory:view.previewEmpty')}
          missingSourceMessage={t('memory:view.previewMissing')}
        />
      </div>
    </div>
  );
}
