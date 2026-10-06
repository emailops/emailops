import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

/** Preference set once the user has closed the notice. */
export const PREF_SHEET_BASICS_NOTICE_DISMISSED = 'sheet_basics_notice_dismissed';

/**
 * A strip above an EO Docs sheet saying sheets are basic for now (text,
 * numbers and a handful of formulas), shown until the user closes it once.
 * It stays hidden while the preference loads, and when it cannot be read.
 */
export function SheetBasicsNotice() {
  const { t } = useTranslation(['documents']);
  const addLog = useLogStore((s) => s.addLog);
  const [show, setShow] = useState(false);

  useEffect(() => {
    let live = true;
    api
      .getPref(PREF_SHEET_BASICS_NOTICE_DISMISSED)
      .then((value) => {
        if (live) setShow(value !== 'true');
      })
      .catch((err) => {
        addLog('error', 'system', `Could not read the sheet notice setting: ${errorText(err)}`);
      });
    return () => {
      live = false;
    };
  }, [addLog]);

  if (!show) return null;

  const close = () => {
    setShow(false);
    api.setPref(PREF_SHEET_BASICS_NOTICE_DISMISSED, 'true').catch((err) => {
      addLog('error', 'system', `Could not save the sheet notice setting: ${errorText(err)}`);
    });
  };

  return (
    <div
      data-testid="sheet-basics-notice"
      className="mb-3 flex items-start justify-between gap-3 rounded border border-gray-600 bg-gray-800/60 px-3 py-2 text-xs text-gray-300"
    >
      <p>{t('documents:sheet.basicsNotice')}</p>
      <button
        type="button"
        data-testid="sheet-basics-notice-close"
        onClick={close}
        className="shrink-0 rounded px-2 py-0.5 text-gray-200 hover:bg-gray-700"
      >
        {t('documents:sheet.basicsNoticeClose')}
      </button>
    </div>
  );
}
