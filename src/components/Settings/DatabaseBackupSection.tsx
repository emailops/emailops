import { type ReactNode, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { formatBytes, formatDateTime } from '@/lib/intl';
import { useLogStore } from '@/stores/logStore';

const FREQUENCIES: readonly api.BackupFrequency[] = ['off', 'daily', 'weekly'];
const KEEP_CHOICES = [1, 2, 3] as const;

const FREQUENCY_LABEL = {
  off: 'settings:privacy.dbBackup.off',
  daily: 'settings:privacy.dbBackup.daily',
  weekly: 'settings:privacy.dbBackup.weekly',
} as const;

/**
 * Settings → Privacy: the copy of the database EmailOps makes in `backups/`
 * shortly after launch — how often, how many copies it keeps, what they take
 * on disk, and deleting them.
 */
export function DatabaseBackupSection() {
  const { t, i18n } = useTranslation(['common', 'settings']);
  const { addLog } = useLogStore();
  const [status, setStatus] = useState<api.BackupStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);

  useEffect(() => {
    api
      .getDbBackups()
      .then(setStatus)
      .catch((e) => {
        setError(errorText(e));
        addLog('error', 'system', `Failed to read the database backups: ${errorText(e)}`);
      });
  }, [addLog]);

  const save = async (frequency: api.BackupFrequency, keep: number) => {
    setError(null);
    setBusy(true);
    try {
      setStatus(await api.setDbBackupSettings(frequency, keep));
      addLog(
        'success',
        'system',
        `Database backup set to ${frequency}, keeping ${keep} cop${keep === 1 ? 'y' : 'ies'}`,
      );
    } catch (e) {
      setError(errorText(e));
      addLog('error', 'system', `Could not save the database backup setting: ${errorText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const deleteAll = async (count: number) => {
    setError(null);
    setBusy(true);
    try {
      setStatus(await api.deleteDbBackups());
      addLog('success', 'system', `Deleted ${count} database backup cop${count === 1 ? 'y' : 'ies'}`);
    } catch (e) {
      setError(errorText(e));
      addLog('error', 'system', `Could not delete the database backups: ${errorText(e)}`);
    } finally {
      setBusy(false);
      setConfirmingDelete(false);
    }
  };

  const size = status ? formatBytes(status.totalBytes, i18n.language) : '';

  return (
    <section>
      <h3 className="text-sm font-semibold text-gray-700 mb-1">{t('settings:privacy.dbBackup.title')}</h3>
      <p className="text-xs text-gray-500 mb-3">{t('settings:privacy.dbBackup.help')}</p>
      {error && (
        <div className="mb-3 p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm" role="alert">
          {error}
        </div>
      )}
      {status && (
        <div className="rounded-lg border border-gray-200 bg-white px-4 py-3 space-y-3">
          <ChoiceRow label={t('settings:privacy.dbBackup.frequency')}>
            {FREQUENCIES.map((frequency) => (
              <Choice
                key={frequency}
                testId={`db-backup-frequency-${frequency}`}
                selected={status.frequency === frequency}
                disabled={busy}
                onClick={() => void save(frequency, status.keep)}
              >
                {t(FREQUENCY_LABEL[frequency])}
              </Choice>
            ))}
          </ChoiceRow>
          <ChoiceRow label={t('settings:privacy.dbBackup.keep')}>
            {KEEP_CHOICES.map((keep) => (
              <Choice
                key={keep}
                testId={`db-backup-keep-${keep}`}
                selected={status.keep === keep}
                disabled={busy}
                onClick={() => void save(status.frequency, keep)}
              >
                {keep}
              </Choice>
            ))}
          </ChoiceRow>
          <p data-testid="db-backup-summary" className="text-xs text-gray-500">
            {status.count === 0
              ? t('settings:privacy.dbBackup.none')
              : t('settings:privacy.dbBackup.summary', {
                  count: status.count,
                  size,
                  date: formatDateTime(status.newestAt, i18n.language),
                })}
          </p>
          {status.count > 0 && !confirmingDelete && (
            <button
              type="button"
              data-testid="db-backup-delete"
              onClick={() => setConfirmingDelete(true)}
              disabled={busy}
              className="text-sm text-red-600 hover:text-red-700 transition-colors disabled:opacity-50"
            >
              {t('settings:privacy.dbBackup.delete')}
            </button>
          )}
          {status.count > 0 && confirmingDelete && (
            <div className="rounded bg-gray-100 px-3 py-2 text-xs text-gray-700">
              <p>{t('settings:privacy.dbBackup.deleteConfirm', { count: status.count, size })}</p>
              <div className="mt-2 flex gap-3">
                <button
                  type="button"
                  data-testid="db-backup-delete-confirm"
                  onClick={() => void deleteAll(status.count)}
                  disabled={busy}
                  className="text-red-600 hover:text-red-700 transition-colors disabled:opacity-50"
                >
                  {t('settings:privacy.dbBackup.deleteConfirmButton')}
                </button>
                <button
                  type="button"
                  data-testid="db-backup-delete-cancel"
                  onClick={() => setConfirmingDelete(false)}
                  disabled={busy}
                  className="text-primary-600 hover:text-primary-700 transition-colors disabled:opacity-50"
                >
                  {t('common:actions.cancel')}
                </button>
              </div>
            </div>
          )}
        </div>
      )}
    </section>
  );
}

// Module-scoped on purpose: a component defined inside another remounts on
// every render (src/CLAUDE.md, "Component Identity").
function ChoiceRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-sm text-gray-700 min-w-[8rem]">{label}</span>
      <div className="flex gap-2" role="radiogroup" aria-label={label}>
        {children}
      </div>
    </div>
  );
}

function Choice({
  selected,
  disabled,
  onClick,
  testId,
  children,
}: {
  selected: boolean;
  disabled: boolean;
  onClick: () => void;
  testId: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      className={`px-3 py-1 text-sm rounded-md border transition-colors disabled:opacity-50 ${
        selected
          ? 'border-primary-500 bg-primary-50 text-gray-900'
          : 'border-gray-200 bg-white text-gray-700 hover:border-gray-300'
      }`}
    >
      {children}
    </button>
  );
}
