import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import type { ConversationView } from '@/lib/conversationView';
import { errorText } from '@/lib/errors';
import { useConversationViewStore } from '@/stores/conversationViewStore';
import { useLogStore } from '@/stores/logStore';

/**
 * "Conversation view": the latest message open with the others expanded one
 * by one (default), or the user's latest own message first with every message
 * open. Stored in the SQLite preferences.
 */
export function ConversationViewSetting() {
  const { t } = useTranslation(['settings']);
  const view = useConversationViewStore((s) => s.view);
  const setView = useConversationViewStore((s) => s.setView);
  const addLog = useLogStore((s) => s.addLog);
  const [error, setError] = useState<string | null>(null);

  const options: { value: ConversationView; label: string }[] = [
    { value: 'per-message', label: t('settings:appearance.conversationViewPerMessage') },
    { value: 'own-first', label: t('settings:appearance.conversationViewOwnFirst') },
  ];

  const change = async (next: ConversationView) => {
    setError(null);
    try {
      await setView(next);
      addLog('success', 'system', `Conversation view: ${next}`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the conversation view setting: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="conversation-view-setting">
      <h3 className="text-sm font-semibold text-gray-700 mb-3">{t('settings:appearance.conversationView')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.conversationViewHelp')}</p>
      <Select
        ariaLabel={t('settings:appearance.conversationView')}
        value={view}
        options={options}
        onChange={(v) => void change(v)}
      />
      {error && <p className="mt-2 text-xs text-red-600">{error}</p>}
    </section>
  );
}
