import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useDefaultSenderStore } from '@/stores/defaultSenderStore';
import { useLogStore } from '@/stores/logStore';

/**
 * "Send new messages from": the account of the selected view (default) or one
 * enabled account. A default that was disabled or removed shows, and acts, as
 * the selected view's account. Stored in the SQLite preferences.
 */
export function DefaultSenderSetting() {
  const { t } = useTranslation(['settings']);
  const accounts = useAccountStore((s) => s.accounts);
  const accountId = useDefaultSenderStore((s) => s.accountId);
  const setAccountId = useDefaultSenderStore((s) => s.setAccountId);
  const addLog = useLogStore((s) => s.addLog);
  const [error, setError] = useState<string | null>(null);

  const enabled = accounts.filter((a) => a.enabled);
  const options = [
    { value: '', label: t('settings:appearance.defaultSenderView') },
    ...enabled.map((a) => ({ value: a.id, label: a.email })),
  ];
  const value = enabled.some((a) => a.id === accountId) ? (accountId ?? '') : '';

  const change = async (next: string) => {
    setError(null);
    try {
      await setAccountId(next || null);
      addLog('success', 'system', `Send new messages from: ${next || 'the selected view'}`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the default sender: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="default-sender-setting">
      <h3 className="text-sm font-semibold text-gray-700 mb-3">{t('settings:appearance.defaultSender')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.defaultSenderHelp')}</p>
      <Select
        ariaLabel={t('settings:appearance.defaultSender')}
        value={value}
        options={options}
        onChange={(v) => void change(v)}
      />
      {error && <p className="mt-2 text-xs text-red-600">{error}</p>}
    </section>
  );
}
