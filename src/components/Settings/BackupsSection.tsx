import { type ReactNode, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { formatBytes, formatDateTime, formatTime } from '@/lib/intl';
import { useLogStore } from '@/stores/logStore';

const FREQUENCIES: readonly api.BackupFrequency[] = ['off', 'daily', 'weekly'];
const KEEP_CHOICES = [1, 2, 3] as const;
const HOURS = Array.from({ length: 24 }, (_, hour) => hour);

const FREQUENCY_LABEL = {
  off: 'settings:privacy.backups.off',
  daily: 'settings:privacy.backups.daily',
  weekly: 'settings:privacy.backups.weekly',
} as const;

/** `hour`:00 as the user's locale writes a time of day. */
function hourLabel(hour: number, locale: string): string {
  return formatTime(new Date(2026, 0, 1, hour).getTime() / 1000, locale);
}

/**
 * Settings → Privacy: the backups EmailOps makes while it is open, at the hour
 * chosen here — the local data copy (settings, rules, memory, chats… which
 * exist only on this computer), which can be restored, and the full database
 * copy, which can be deleted.
 */
export function BackupsSection() {
  const { t, i18n } = useTranslation(['common', 'settings']);
  const { addLog } = useLogStore();
  const [status, setStatus] = useState<api.BackupStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmingDelete, setConfirmingDelete] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);

  useEffect(() => {
    api
      .getBackups()
      .then(setStatus)
      .catch((e) => {
        setError(errorText(e));
        addLog('error', 'system', `Failed to read the backups: ${errorText(e)}`);
      });
  }, [addLog]);

  const save = async (current: api.BackupStatus, change: Partial<api.BackupSettings>) => {
    const settings = { hour: current.hour, local: current.local, full: current.full, keep: current.keep, ...change };
    setError(null);
    setBusy(true);
    try {
      setStatus(await api.setBackupSettings(settings));
      addLog(
        'success',
        'system',
        `Backups set to ${settings.hour}:00: local data ${settings.local}, full copy ${settings.full}, keeping ${settings.keep} cop${settings.keep === 1 ? 'y' : 'ies'}`,
      );
    } catch (e) {
      setError(errorText(e));
      addLog('error', 'system', `Could not save the backup settings: ${errorText(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const deleteFull = async (count: number) => {
    setError(null);
    setBusy(true);
    try {
      setStatus(await api.deleteFullBackups());
      addLog('success', 'system', `Deleted ${count} database backup cop${count === 1 ? 'y' : 'ies'}`);
    } catch (e) {
      setError(errorText(e));
      addLog('error', 'system', `Could not delete the database backups: ${errorText(e)}`);
    } finally {
      setBusy(false);
      setConfirmingDelete(false);
    }
  };

  // On success the app restarts, so only a failure comes back here.
  const restore = async (name: string) => {
    setError(null);
    setBusy(true);
    try {
      await api.restoreLocalBackup(name);
    } catch (e) {
      setError(errorText(e));
      addLog('error', 'system', `Could not restore the local data: ${errorText(e)}`);
      setBusy(false);
      setRestoring(null);
    }
  };

  const summary = (copies: api.BackupCopy[]) =>
    copies.length === 0
      ? t('settings:privacy.backups.none')
      : t('settings:privacy.backups.summary', {
          count: copies.length,
          size: formatBytes(
            copies.reduce((sum, c) => sum + c.bytes, 0),
            i18n.language,
          ),
          date: formatDateTime(copies[0].takenAt, i18n.language),
        });

  const fullSize = status
    ? formatBytes(
        status.fullCopies.reduce((sum, c) => sum + c.bytes, 0),
        i18n.language,
      )
    : '';
  const restoringCopy = status?.localCopies.find((c) => c.name === restoring);

  return (
    <section>
      <h3 className="text-sm font-semibold text-gray-700 mb-1">{t('settings:privacy.backups.title')}</h3>
      <p className="text-xs text-gray-500 mb-3">{t('settings:privacy.backups.help')}</p>
      {error && (
        <div className="mb-3 p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm" role="alert">
          {error}
        </div>
      )}
      {status && (
        <div className="rounded-lg border border-gray-200 bg-white px-4 py-3 space-y-4">
          <Row label={t('settings:privacy.backups.time')}>
            <Select
              testId="backup-hour"
              ariaLabel={t('settings:privacy.backups.time')}
              value={String(status.hour)}
              options={HOURS.map((hour) => ({ value: String(hour), label: hourLabel(hour, i18n.language) }))}
              disabled={busy}
              onChange={(v) => void save(status, { hour: Number(v) })}
            />
          </Row>

          <div className="space-y-2">
            <h4 className="text-sm font-medium text-gray-900">{t('settings:privacy.backups.localTitle')}</h4>
            <p className="text-xs text-gray-500">{t('settings:privacy.backups.localHelp')}</p>
            <Row label={t('settings:privacy.backups.frequency')} radio>
              {FREQUENCIES.map((frequency) => (
                <Choice
                  key={frequency}
                  testId={`backup-local-${frequency}`}
                  selected={status.local === frequency}
                  disabled={busy}
                  onClick={() => void save(status, { local: frequency })}
                >
                  {t(FREQUENCY_LABEL[frequency])}
                </Choice>
              ))}
            </Row>
            <p data-testid="backup-local-summary" className="text-xs text-gray-500">
              {summary(status.localCopies)}
            </p>
            {status.localCopies.length > 0 && restoring === null && (
              <button
                type="button"
                data-testid="backup-restore"
                onClick={() => setRestoring(status.localCopies[0].name)}
                disabled={busy}
                className="text-sm text-primary-600 hover:text-primary-700 transition-colors disabled:opacity-50"
              >
                {t('settings:privacy.backups.restore')}
              </button>
            )}
            {restoringCopy && (
              <div className="rounded bg-gray-100 px-3 py-2 text-xs text-gray-700 space-y-2">
                <Select
                  testId="backup-restore-copy"
                  ariaLabel={t('settings:privacy.backups.restoreCopy')}
                  size="xs"
                  value={restoringCopy.name}
                  options={status.localCopies.map((c) => ({
                    value: c.name,
                    label: formatDateTime(c.takenAt, i18n.language),
                  }))}
                  disabled={busy}
                  onChange={setRestoring}
                />
                <p>
                  {t('settings:privacy.backups.restoreConfirm', {
                    date: formatDateTime(restoringCopy.takenAt, i18n.language),
                  })}
                </p>
                <div className="flex gap-3">
                  <button
                    type="button"
                    data-testid="backup-restore-confirm"
                    onClick={() => void restore(restoringCopy.name)}
                    disabled={busy}
                    className="text-primary-600 hover:text-primary-700 transition-colors disabled:opacity-50"
                  >
                    {t('settings:privacy.backups.restoreConfirmButton')}
                  </button>
                  <button
                    type="button"
                    data-testid="backup-restore-cancel"
                    onClick={() => setRestoring(null)}
                    disabled={busy}
                    className="text-gray-600 hover:text-gray-700 transition-colors disabled:opacity-50"
                  >
                    {t('common:actions.cancel')}
                  </button>
                </div>
              </div>
            )}
          </div>

          <div className="space-y-2">
            <h4 className="text-sm font-medium text-gray-900">{t('settings:privacy.backups.fullTitle')}</h4>
            <p className="text-xs text-gray-500">{t('settings:privacy.backups.fullHelp')}</p>
            <Row label={t('settings:privacy.backups.frequency')} radio>
              {FREQUENCIES.map((frequency) => (
                <Choice
                  key={frequency}
                  testId={`backup-full-${frequency}`}
                  selected={status.full === frequency}
                  disabled={busy}
                  onClick={() => void save(status, { full: frequency })}
                >
                  {t(FREQUENCY_LABEL[frequency])}
                </Choice>
              ))}
            </Row>
            <Row label={t('settings:privacy.backups.keep')} radio>
              {KEEP_CHOICES.map((keep) => (
                <Choice
                  key={keep}
                  testId={`backup-keep-${keep}`}
                  selected={status.keep === keep}
                  disabled={busy}
                  onClick={() => void save(status, { keep })}
                >
                  {keep}
                </Choice>
              ))}
            </Row>
            <p data-testid="backup-full-summary" className="text-xs text-gray-500">
              {summary(status.fullCopies)}
            </p>
            {status.fullCopies.length > 0 && !confirmingDelete && (
              <button
                type="button"
                data-testid="backup-delete"
                onClick={() => setConfirmingDelete(true)}
                disabled={busy}
                className="text-sm text-red-600 hover:text-red-700 transition-colors disabled:opacity-50"
              >
                {t('settings:privacy.backups.delete')}
              </button>
            )}
            {status.fullCopies.length > 0 && confirmingDelete && (
              <div className="rounded bg-gray-100 px-3 py-2 text-xs text-gray-700">
                <p>
                  {t('settings:privacy.backups.deleteConfirm', { count: status.fullCopies.length, size: fullSize })}
                </p>
                <div className="mt-2 flex gap-3">
                  <button
                    type="button"
                    data-testid="backup-delete-confirm"
                    onClick={() => void deleteFull(status.fullCopies.length)}
                    disabled={busy}
                    className="text-red-600 hover:text-red-700 transition-colors disabled:opacity-50"
                  >
                    {t('settings:privacy.backups.deleteConfirmButton')}
                  </button>
                  <button
                    type="button"
                    data-testid="backup-delete-cancel"
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
        </div>
      )}
    </section>
  );
}

// Module-scoped on purpose: a component defined inside another remounts on
// every render (src/CLAUDE.md, "Component Identity").
function Row({ label, radio = false, children }: { label: string; radio?: boolean; children: ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-sm text-gray-700 min-w-[8rem]">{label}</span>
      {radio ? (
        <div className="flex gap-2" role="radiogroup" aria-label={label}>
          {children}
        </div>
      ) : (
        children
      )}
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
