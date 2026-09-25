// Error detail for one Lens run, collapsed by default: the run's own error
// (why it stopped) and each email that failed while it ran, with the reason.
// Loaded on first open — most runs are never expanded.

import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { LensRunFailure, LensRunHistoryEntry } from '@/types';

interface LensRunErrorsProps {
  lensId: string;
  run: LensRunHistoryEntry;
}

export function LensRunErrors({ lensId, run }: LensRunErrorsProps) {
  const { t } = useTranslation(['lenses']);
  const [failures, setFailures] = useState<LensRunFailure[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const onToggle = (e: React.SyntheticEvent<HTMLDetailsElement>) => {
    if (!e.currentTarget.open || failures !== null) return;
    api
      .listLensRunFailures(lensId, run.id)
      .then(setFailures)
      .catch((err: unknown) => setLoadError(errorText(err)));
  };

  return (
    <details onToggle={onToggle} className="text-[11px] text-gray-400">
      <summary className="cursor-pointer select-none text-red-300/80 hover:text-red-300">
        {run.failed > 0
          ? t('lenses:runHistory.errorsSummary', { count: run.failed })
          : t('lenses:runHistory.runErrorSummary')}
      </summary>
      <div className="mt-1.5 space-y-1.5 pl-3">
        {run.errorMessage && (
          <p>
            <span className="text-gray-500">{t('lenses:runHistory.runError')}:</span>{' '}
            <span className="text-red-300">{run.errorMessage}</span>
          </p>
        )}
        {loadError && <p className="text-red-400">{loadError}</p>}
        {failures === null && !loadError && run.failed > 0 && <p>{t('lenses:runHistory.loading')}</p>}
        {failures !== null && failures.length === 0 && run.failed > 0 && (
          <p className="italic">{t('lenses:runHistory.failuresRetried')}</p>
        )}
        {failures && failures.length > 0 && (
          <ul className="space-y-1">
            {failures.map((f) => (
              <li key={f.emailId} className="rounded border border-gray-800 px-2 py-1">
                <div className="truncate text-gray-200" title={f.subject}>
                  {f.subject || t('lenses:create.noSubject')} <span className="text-gray-500">— {f.sender}</span>
                </div>
                <div className="break-words text-red-300">{f.errorMessage || t('lenses:runHistory.unknownError')}</div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </details>
  );
}
