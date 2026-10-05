import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import type { AfterLeaveMode } from '@/lib/autoAdvance';
import { errorText } from '@/lib/errors';
import { useAutoAdvanceStore } from '@/stores/autoAdvanceStore';
import { useLogStore } from '@/stores/logStore';

/**
 * "After archiving or deleting": open the next conversation (default), the
 * previous one, or go back to the list. Stored in the SQLite preferences.
 */
export function AutoAdvanceSetting() {
  const { t } = useTranslation(['settings']);
  const mode = useAutoAdvanceStore((s) => s.mode);
  const setMode = useAutoAdvanceStore((s) => s.setMode);
  const addLog = useLogStore((s) => s.addLog);
  const [error, setError] = useState<string | null>(null);

  const options: { value: AfterLeaveMode; label: string }[] = [
    { value: 'next', label: t('settings:appearance.afterLeaveNext') },
    { value: 'previous', label: t('settings:appearance.afterLeavePrevious') },
    { value: 'list', label: t('settings:appearance.afterLeaveList') },
  ];

  const change = async (next: AfterLeaveMode) => {
    setError(null);
    try {
      await setMode(next);
      addLog('success', 'system', `After archiving or deleting: ${next}`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the auto-advance setting: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="auto-advance-setting">
      <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:appearance.afterLeave')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.afterLeaveHelp')}</p>
      <Select
        ariaLabel={t('settings:appearance.afterLeave')}
        value={mode}
        options={options}
        onChange={(v) => void change(v)}
      />
      {error && <p className="mt-2 text-xs text-red-400">{error}</p>}
    </section>
  );
}
