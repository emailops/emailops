import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import type { StorageStats } from '@/types';

interface StorageCardProps {
  stats: StorageStats | null;
  error: string | null;
}

/**
 * Convert bytes to a human-readable string with binary-prefix units (KiB/MiB/GiB).
 * Uses 1024 so reported sizes match `du -h` / Finder "Get Info" on macOS.
 */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let val = n / 1024;
  let i = 0;
  while (val >= 1024 && i < units.length - 1) {
    val /= 1024;
    i += 1;
  }
  return `${val.toFixed(val >= 100 ? 0 : val >= 10 ? 1 : 2)} ${units[i]}`;
}

function pct(part: number, total: number): string {
  if (total === 0) return '0%';
  return `${((part / total) * 100).toFixed(1)}%`;
}

interface Row {
  label: string;
  bytes: number;
  hint?: string;
}

export function StorageCard({ stats, error }: StorageCardProps) {
  const { t } = useTranslation(['dashboard']);
  const fmt = useFormatters();
  if (error) {
    return (
      <div className="border border-red-800 bg-red-950 text-red-200 text-sm rounded p-3">
        Failed to load storage stats: {error}
      </div>
    );
  }
  if (!stats) {
    return <div className="text-xs text-gray-500 italic">{t('dashboard:storage.loading')}</div>;
  }

  const rows: Row[] = [
    { label: 'SQLite database', bytes: stats.dbFileBytes, hint: 'emailops.db' },
    { label: 'Write-ahead log', bytes: stats.walBytes, hint: 'emailops.db-wal' },
    { label: 'Shared memory', bytes: stats.shmBytes, hint: 'emailops.db-shm' },
    { label: 'Attachments on disk', bytes: stats.attachmentsBytes, hint: 'attachments/' },
    { label: 'Local AI models', bytes: stats.modelsBytes, hint: 'models/' },
    { label: 'DB backups', bytes: stats.backupsBytes, hint: 'backups/' },
    { label: 'Other', bytes: stats.otherBytes, hint: 'locks, scratch' },
  ];

  return (
    <div className="border border-gray-800 bg-gray-900 rounded p-4">
      <div className="flex items-baseline justify-between mb-3">
        <div>
          <div className="text-xs uppercase tracking-wider text-gray-500">{t('dashboard:storage.totalOnDisk')}</div>
          <div className="text-2xl font-semibold text-white">{formatBytes(stats.totalBytes)}</div>
        </div>
        <div className="text-xs text-gray-500">{fmt.dateTime(stats.computedAt)}</div>
      </div>
      <div className="space-y-1.5">
        {rows.map((row) => {
          const width = stats.totalBytes === 0 ? 0 : (row.bytes / stats.totalBytes) * 100;
          return (
            <div key={row.label} className="text-sm">
              <div className="flex items-baseline justify-between text-gray-300">
                <span>
                  {row.label}
                  {row.hint && <span className="ml-2 text-xs text-gray-600">{row.hint}</span>}
                </span>
                <span className="text-gray-400 tabular-nums">
                  {formatBytes(row.bytes)}
                  <span className="ml-2 text-xs text-gray-600">{pct(row.bytes, stats.totalBytes)}</span>
                </span>
              </div>
              <div className="mt-0.5 h-1 bg-gray-800 rounded overflow-hidden">
                <div className="h-full bg-primary-600" style={{ width: `${width}%` }} />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
