import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { errorText } from '@/lib/errors';
import { UNDO_SEND_DELAY_OPTIONS } from '@/lib/outbox';
import { useLogStore } from '@/stores/logStore';
import { useOutboxStore } from '@/stores/outboxStore';

/**
 * "Undo send": how long a sent message waits before it leaves, so it can be
 * taken back (off / 5 / 10 / 20 / 30 s). Stored in the SQLite preferences.
 */
export function UndoSendSetting() {
  const { t } = useTranslation(['settings']);
  const loadUndoDelay = useOutboxStore((s) => s.loadUndoDelay);
  const setUndoDelay = useOutboxStore((s) => s.setUndoDelay);
  const addLog = useLogStore((s) => s.addLog);
  const [value, setValue] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void loadUndoDelay().then(setValue);
  }, [loadUndoDelay]);

  const options = UNDO_SEND_DELAY_OPTIONS.map((secs) => ({
    value: String(secs),
    label:
      secs === 0 ? t('settings:appearance.undoSendOff') : t('settings:appearance.undoSendSeconds', { count: secs }),
  }));

  const change = async (raw: string) => {
    const secs = Number(raw);
    setError(null);
    try {
      await setUndoDelay(secs);
      setValue(secs);
      addLog('success', 'system', `Undo send set to ${secs}s`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the undo-send setting: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="undo-send-setting">
      <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:appearance.undoSend')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.undoSendHelp')}</p>
      {value !== null && (
        <Select
          ariaLabel={t('settings:appearance.undoSend')}
          value={String(value)}
          options={options}
          onChange={(v) => void change(v)}
        />
      )}
      {error && <p className="mt-2 text-xs text-red-400">{error}</p>}
    </section>
  );
}
