import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

/** SQLite preference the send service reads when a message leaves (`APPEND_FOOTER_PREF`). */
const APPEND_FOOTER_PREF = 'compose.append_footer';

/**
 * Whether sent messages end with the "Sent with EmailOps" line, for every
 * account. On unless the stored value is `false`.
 */
export function SentFooterSetting() {
  const { t } = useTranslation(['settings']);
  const addLog = useLogStore((s) => s.addLog);
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    api
      .getPref(APPEND_FOOTER_PREF)
      .then((value) => {
        if (!stale) setEnabled(value !== 'false');
      })
      .catch((err) => {
        if (stale) return;
        setError(errorText(err));
        addLog('error', 'system', `Could not read the footer setting: ${errorText(err)}`);
      });
    return () => {
      stale = true;
    };
  }, [addLog]);

  const change = async (next: boolean) => {
    setError(null);
    try {
      await api.setPref(APPEND_FOOTER_PREF, String(next));
      setEnabled(next);
      addLog('success', 'system', `"Sent with EmailOps" footer ${next ? 'on' : 'off'}`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the footer setting: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="sent-footer-setting" className="border-t border-gray-200 pt-4 space-y-1">
      <label className="flex items-center gap-2 text-sm text-gray-700">
        <input
          type="checkbox"
          checked={enabled ?? true}
          disabled={enabled === null}
          onChange={(e) => void change(e.target.checked)}
        />
        {t('settings:signatures.sentFooter')}
      </label>
      <p className="text-xs text-gray-500">{t('settings:signatures.sentFooterHelp')}</p>
      {error && <p className="text-xs text-red-600">{error}</p>}
    </section>
  );
}
