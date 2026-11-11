// Modal listing the most recent runs for a Lens (most recent first).
// Sourced from the `lens_runs` table via `list_lens_runs`.

import { Fragment, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { formatDayMonthYearTime } from '@/lib/intl';
import type { LensRunHistoryEntry } from '@/types';
import { LensRunErrors } from './LensRunErrors';

interface LensRunHistoryDialogProps {
  lensId: string | null;
  lensName: string;
  open: boolean;
  onClose: () => void;
}

export function LensRunHistoryDialog({ lensId, lensName, open, onClose }: LensRunHistoryDialogProps) {
  const { t } = useTranslation(['common', 'lenses']);
  const [runs, setRuns] = useState<LensRunHistoryEntry[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !lensId) return;
    let cancelled = false;
    setIsLoading(true);
    setError(null);
    api
      .listLensRuns(lensId, 50)
      .then((rs) => {
        if (!cancelled) setRuns(rs);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(errorText(e));
      })
      .finally(() => {
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [open, lensId]);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={t('lenses:runHistory.title', { name: lensName })}
      size="2xl"
      footer={
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-gray-300 px-3 py-1 text-xs text-gray-800 hover:bg-gray-100"
          >
            {t('common:actions.close')}
          </button>
        </div>
      }
    >
      {isLoading && <div className="p-4 text-xs text-gray-500">{t('lenses:runHistory.loading')}</div>}
      {error && <div className="p-4 text-xs text-red-600">{error}</div>}
      {!isLoading && !error && runs.length === 0 && (
        <div className="p-4 text-xs text-gray-500">{t('lenses:runHistory.empty')}</div>
      )}
      {runs.length > 0 && (
        <table className="w-full text-left text-xs">
          <thead className="text-gray-500">
            <tr>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.started')}</th>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.kind')}</th>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.status')}</th>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.processed')}</th>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.ok')}</th>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.failed')}</th>
              <th className="px-3 py-2 font-medium">{t('lenses:runHistory.duration')}</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((r) => (
              <Fragment key={r.id}>
                <tr className="border-t border-gray-200">
                  <td className="px-3 py-2 align-top text-gray-700">{formatDayMonthYearTime(r.startedAt)}</td>
                  <td className="px-3 py-2 align-top text-gray-700">
                    {t(`lenses:runHistory.kinds.${r.kind}`, { defaultValue: r.kind })}
                  </td>
                  <td className="px-3 py-2 align-top">
                    <StatusBadge status={r.status} />
                  </td>
                  <td className="px-3 py-2 align-top text-gray-700">{r.processed}</td>
                  <td className="px-3 py-2 align-top text-green-700">{r.succeeded}</td>
                  <td className="px-3 py-2 align-top text-red-700">{r.failed}</td>
                  <td className="px-3 py-2 align-top text-gray-500">{formatDuration(r)}</td>
                </tr>
                {lensId && (r.failed > 0 || r.errorMessage) && (
                  <tr>
                    <td colSpan={7} className="px-3 pb-2">
                      <LensRunErrors lensId={lensId} run={r} />
                    </td>
                  </tr>
                )}
              </Fragment>
            ))}
          </tbody>
        </table>
      )}
    </Modal>
  );
}

function StatusBadge({ status }: { status: string }) {
  const { t } = useTranslation(['lenses']);
  const cls =
    status === 'success'
      ? 'border-green-200 text-green-700 bg-green-50'
      : status === 'failed'
        ? 'border-red-200 text-red-700 bg-red-50'
        : status === 'cancelled'
          ? 'border-yellow-200 text-yellow-700 bg-yellow-50'
          : status === 'running'
            ? 'border-blue-200 text-blue-700 bg-blue-50'
            : 'border-gray-200 text-gray-700';
  return (
    <span className={`inline-block rounded border px-1.5 py-0.5 text-[10px] ${cls}`}>
      {t(`lenses:runHistory.statuses.${status}`, { defaultValue: status })}
    </span>
  );
}

function formatDuration(r: LensRunHistoryEntry): string {
  if (!r.finishedAt) return '—';
  const secs = Math.max(0, r.finishedAt - r.startedAt);
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s}s`;
}
