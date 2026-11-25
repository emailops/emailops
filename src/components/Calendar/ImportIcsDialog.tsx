import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import { Select } from '@/components/shared/Select';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import type { CreateAccountChoice } from '@/lib/calendarAccounts';
import { describeRecurrence } from '@/lib/calendarEvent';
import { errorText, isAuthError } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { IcsImportPreview, IcsImportPreviewItem, IcsImportSummary } from '@/types';

interface ImportIcsDialogProps {
  /** Accounts the file can go to; only the in-app writable ones are offered. */
  choices: CreateAccountChoice[];
  /** Account selected in the view; the first choice when it is not one. */
  initialAccountId: string;
  onClose: () => void;
  /** The import finished (even partly) — the view refreshes the calendar. */
  onImported: (accountId: string) => void;
  /** Auth-class failure — the view shows the account's re-auth banner. */
  onAuthError: (accountId: string) => void;
}

/** "Import .ics": choose a file, preview what each event would do, import it
 *  into the account's primary calendar and show the summary. */
export function ImportIcsDialog({ choices, initialAccountId, onClose, onImported, onAuthError }: ImportIcsDialogProps) {
  const { t, i18n } = useTranslation(['calendar', 'common']);
  const { date, time, dateTime } = useFormatters();
  const addLog = useLogStore((s) => s.addLog);

  const [accountId, setAccountId] = useState(
    () => (choices.find((c) => c.account.id === initialAccountId) ?? choices[0]).account.id,
  );
  const [path, setPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<IcsImportPreview | null>(null);
  const [isReading, setIsReading] = useState(false);
  const [isImporting, setIsImporting] = useState(false);
  const [summary, setSummary] = useState<IcsImportSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const previewReqRef = useRef(0);
  const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone;

  const fail = (e: unknown, forAccount: string) => {
    const msg = errorText(e);
    addLog('error', 'sync', `Failed to import calendar file: ${msg}`);
    if (isAuthError(e, msg)) onAuthError(forAccount);
    else setError(msg);
  };

  // A new file or account asks for a new preview; an older answer is dropped.
  useEffect(() => {
    if (!path) return;
    const reqId = ++previewReqRef.current;
    setIsReading(true);
    setPreview(null);
    setError(null);
    api
      .previewIcsImport(accountId, path, timeZone)
      .then((result) => {
        if (reqId === previewReqRef.current) setPreview(result);
      })
      .catch((e) => {
        if (reqId === previewReqRef.current) fail(e, accountId);
      })
      .finally(() => {
        if (reqId === previewReqRef.current) setIsReading(false);
      });
  }, [path, accountId, timeZone]);

  const chooseFile = async () => {
    try {
      const picked = await openFileDialog({ multiple: false, filters: [{ name: 'iCalendar', extensions: ['ics'] }] });
      if (typeof picked === 'string') setPath(picked);
    } catch (e) {
      fail(e, accountId);
    }
  };

  // The dialog opens straight onto the picker; a cancelled picker leaves the
  // "Choose file…" button. The ref keeps StrictMode's second mount from
  // opening a second native picker.
  const pickerAskedRef = useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs once, on mount
  useEffect(() => {
    if (pickerAskedRef.current) return;
    pickerAskedRef.current = true;
    void chooseFile();
  }, []);

  const importable = preview?.items.filter((i) => i.status === 'importable').length ?? 0;
  const busy = isReading || isImporting;

  const runImport = async () => {
    if (!path || busy || importable === 0) return;
    setIsImporting(true);
    setError(null);
    addLog('info', 'sync', 'Importing calendar file');
    try {
      const result = await api.importIcs(accountId, path, timeZone);
      addLog(
        'success',
        'sync',
        `Calendar file imported: ${result.imported} imported, ${result.alreadyPresent} already present, ${result.unsupported} unsupported, ${result.failed.length} failed`,
      );
      setSummary(result);
      onImported(accountId);
    } catch (e) {
      fail(e, accountId);
    } finally {
      setIsImporting(false);
    }
  };

  // "Tue 1 Dec · 10:00 – 11:00 · Weekly on Tuesday" — the invite card's shape.
  const whenLine = (item: IcsImportPreviewItem) => {
    const day = date(item.startTime, { weekday: 'short', day: 'numeric', month: 'short' });
    const sameDay = new Date(item.startTime * 1000).toDateString() === new Date(item.endTime * 1000).toDateString();
    const when = item.isAllDay
      ? `${day} · ${t('calendar:import.allDay')}`
      : item.endTime <= item.startTime
        ? `${day} · ${time(item.startTime)}`
        : sameDay
          ? `${day} · ${time(item.startTime)} – ${time(item.endTime)}`
          : `${dateTime(item.startTime)} – ${dateTime(item.endTime)}`;
    if (!item.recurrence) return when;
    const rule = describeRecurrence(item.recurrence, new Date(item.startTime * 1000), i18n.language || 'en');
    return `${when} · ${rule.labelKey ? t(`calendar:create.recurrence.${rule.labelKey}`, rule.params ?? {}) : rule.raw}`;
  };

  const footer = summary ? (
    <button
      type="button"
      onClick={onClose}
      className="px-4 py-1.5 text-sm font-medium rounded-md bg-primary-600 text-white hover:bg-primary-700"
    >
      {t('common:actions.close')}
    </button>
  ) : (
    <>
      <button
        type="button"
        onClick={onClose}
        className="px-4 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50"
      >
        {t('common:actions.cancel')}
      </button>
      <button
        type="button"
        onClick={() => void runImport()}
        disabled={busy || importable === 0}
        className="px-4 py-1.5 text-sm font-medium rounded-md bg-primary-600 text-white hover:bg-primary-700 disabled:opacity-50"
      >
        {isImporting ? t('calendar:import.importing') : t('calendar:import.importN', { count: importable })}
      </button>
    </>
  );

  return (
    <Modal open onClose={onClose} title={t('calendar:import.title')} size="lg" footer={footer}>
      {/* Error banner — first and sticky, never below the list. */}
      {error && (
        <div
          role="alert"
          className="sticky top-0 z-10 -mt-1 mb-3 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800 break-words"
        >
          {t('calendar:import.error', { message: error })}
        </div>
      )}

      {summary ? (
        <div className="space-y-2 text-sm text-gray-700">
          <p className="font-semibold text-gray-900">{t('calendar:import.summary.title')}</p>
          <p>{t('calendar:import.summary.imported', { count: summary.imported })}</p>
          {summary.alreadyPresent > 0 && (
            <p>{t('calendar:import.summary.alreadyPresent', { count: summary.alreadyPresent })}</p>
          )}
          {summary.unsupported > 0 && <p>{t('calendar:import.summary.unsupported', { count: summary.unsupported })}</p>}
          {summary.failed.length > 0 && (
            <div>
              <p className="text-red-700">{t('calendar:import.summary.failed', { count: summary.failed.length })}</p>
              <ul className="mt-1 space-y-1">
                {summary.failed.map((f, i) => (
                  // Titles can repeat; the list never reorders.
                  // biome-ignore lint/suspicious/noArrayIndexKey: static list
                  <li key={i} className="text-xs text-gray-600 break-words">
                    {f.title} — {f.reason}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      ) : (
        <div className="space-y-4">
          {choices.length > 1 && (
            <div>
              <label className="block text-xs font-semibold text-gray-500 uppercase tracking-wider mb-1">
                {t('calendar:import.account')}
              </label>
              <Select
                value={accountId}
                onChange={setAccountId}
                options={choices.map((c) => ({ value: c.account.id, label: c.account.email }))}
                ariaLabel={t('calendar:import.account')}
                fullWidth
                disabled={isImporting}
              />
            </div>
          )}

          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => void chooseFile()}
              disabled={isImporting}
              className="px-3 py-1.5 text-sm border border-gray-300 rounded-md text-gray-700 hover:bg-gray-50 disabled:opacity-50"
            >
              {path ? t('calendar:import.chooseAnother') : t('calendar:import.choose')}
            </button>
            {preview && (
              <span className="min-w-0 text-xs text-gray-500 break-all">
                {t('calendar:import.fileName', { name: preview.fileName })}
              </span>
            )}
          </div>

          {!path && <p className="text-sm text-gray-500">{t('calendar:import.noFile')}</p>}
          {isReading && <p className="text-sm text-gray-500">{t('calendar:import.loading')}</p>}
          {preview && preview.items.length === 0 && (
            <p className="text-sm text-gray-500">{t('calendar:import.empty')}</p>
          )}
          {preview && preview.items.length > 0 && (
            <ul className="divide-y divide-gray-100 border border-gray-200 rounded-md">
              {preview.items.map((item, i) => (
                // UIDs can be empty or repeat; the list never reorders.
                // biome-ignore lint/suspicious/noArrayIndexKey: static list
                <li key={i} className="px-3 py-2">
                  <p className="text-sm font-medium text-gray-900 break-words">{item.title}</p>
                  {/* An item without a start has no date to show. */}
                  {item.reason !== 'missingUidOrStart' && <p className="text-xs text-gray-500">{whenLine(item)}</p>}
                  <p className={`text-xs ${item.status === 'unsupported' ? 'text-amber-700' : 'text-gray-600'}`}>
                    {item.status === 'unsupported' && item.reason
                      ? `${t(`calendar:import.reason.${item.reason}` as const, { defaultValue: item.reason })}${
                          item.reasonDetail ? ` (${item.reasonDetail})` : ''
                        }`
                      : t(`calendar:import.status.${item.status}` as const)}
                  </p>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </Modal>
  );
}
