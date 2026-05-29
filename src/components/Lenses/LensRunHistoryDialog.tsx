// Modal listing the most recent runs for a Lens (most recent first).
// Sourced from the `lens_runs` table via `list_lens_runs`.

import { format } from 'date-fns';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { Modal } from '@/components/common/Modal';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { LensRunHistoryEntry } from '@/types';

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
      title={`Run history — ${lensName}`}
      size="lg"
      footer={
        <div className="flex justify-end">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-gray-600 px-3 py-1 text-xs text-gray-200 hover:bg-gray-700"
          >
            {t('common:actions.close')}
          </button>
        </div>
      }
    >
      {isLoading && <div className="p-4 text-xs text-gray-400">{t('lenses:runHistory.loading')}</div>}
      {error && <div className="p-4 text-xs text-red-400">{error}</div>}
      {!isLoading && !error && runs.length === 0 && (
        <div className="p-4 text-xs text-gray-400">{t('lenses:runHistory.empty')}</div>
      )}
      {runs.length > 0 && (
        <table className="w-full text-left text-xs">
          <thead className="text-gray-400">
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
              <tr key={r.id} className="border-t border-gray-800">
                <td className="px-3 py-2 align-top text-gray-300">
                  {format(new Date(r.startedAt * 1000), 'MMM d, yyyy h:mm a')}
                </td>
                <td className="px-3 py-2 align-top text-gray-300">{r.kind}</td>
                <td className="px-3 py-2 align-top">
                  <StatusBadge status={r.status} />
                </td>
                <td className="px-3 py-2 align-top text-gray-300">{r.processed}</td>
                <td className="px-3 py-2 align-top text-green-300">{r.succeeded}</td>
                <td className="px-3 py-2 align-top text-red-300">{r.failed}</td>
                <td className="px-3 py-2 align-top text-gray-400">{formatDuration(r)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {runs.some((r) => r.errorMessage) && (
        <details className="mt-3 px-3 text-[11px] text-gray-400">
          <summary className="cursor-pointer">{t('lenses:runHistory.errorMessages')}</summary>
          <ul className="mt-2 space-y-1">
            {runs
              .filter((r) => r.errorMessage)
              .map((r) => (
                <li key={r.id}>
                  <span className="text-gray-500">{format(new Date(r.startedAt * 1000), 'MMM d, h:mm a')}:</span>{' '}
                  <span className="text-red-300">{r.errorMessage}</span>
                </li>
              ))}
          </ul>
        </details>
      )}
    </Modal>
  );
}

function StatusBadge({ status }: { status: string }) {
  const cls =
    status === 'success'
      ? 'border-green-700/60 text-green-300 bg-green-900/30'
      : status === 'failed'
        ? 'border-red-700/60 text-red-300 bg-red-900/30'
        : status === 'cancelled'
          ? 'border-yellow-700/60 text-yellow-300 bg-yellow-900/30'
          : status === 'running'
            ? 'border-blue-700/60 text-blue-300 bg-blue-900/30'
            : 'border-gray-700/60 text-gray-300';
  return <span className={`inline-block rounded border px-1.5 py-0.5 text-[10px] ${cls}`}>{status}</span>;
}

function formatDuration(r: LensRunHistoryEntry): string {
  if (!r.finishedAt) return '—';
  const secs = Math.max(0, r.finishedAt - r.startedAt);
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return `${m}m ${s}s`;
}
