import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { errorText } from '@/lib/errors';
import { UNDO_WINDOW_OPTIONS } from '@/lib/undoWindow';
import { useLogStore } from '@/stores/logStore';
import { useUndoWindowStore } from '@/stores/undoWindowStore';

/**
 * "Undo archive and delete": how long archive and delete wait for an Undo
 * before reaching the mail server (off / 6 / 10 / 20 / 30 s). Separate from
 * Undo send. Stored in the SQLite preferences.
 */
export function UndoWindowSetting() {
  const { t } = useTranslation(['settings']);
  const secs = useUndoWindowStore((s) => s.secs);
  const setSecs = useUndoWindowStore((s) => s.setSecs);
  const addLog = useLogStore((s) => s.addLog);
  const [error, setError] = useState<string | null>(null);

  const options = UNDO_WINDOW_OPTIONS.map((value) => ({
    value: String(value),
    label:
      value === 0
        ? t('settings:appearance.undoArchiveOff')
        : t('settings:appearance.undoArchiveSeconds', { count: value }),
  }));

  const change = async (raw: string) => {
    const next = Number(raw);
    setError(null);
    try {
      await setSecs(next);
      addLog('success', 'system', `Undo archive and delete set to ${next}s`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the undo archive and delete setting: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="undo-window-setting">
      <h3 className="text-sm font-semibold text-gray-700 mb-3">{t('settings:appearance.undoArchive')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.undoArchiveHelp')}</p>
      <Select
        ariaLabel={t('settings:appearance.undoArchive')}
        value={String(secs)}
        options={options}
        onChange={(v) => void change(v)}
      />
      {error && <p className="mt-2 text-xs text-red-600">{error}</p>}
    </section>
  );
}
