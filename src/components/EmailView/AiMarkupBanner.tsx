import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

interface AiMarkupBannerProps {
  accountId: string;
  emailId: string;
}

/**
 * Warning above a message that carries text addressed to the AI assistant
 * (chat-template turn markers, the prompt fence tag). Shown whether or not
 * junk detection is on: the backend checks the body when the message opens.
 */
export function AiMarkupBanner({ accountId, emailId }: AiMarkupBannerProps) {
  const { t } = useTranslation(['inbox']);
  const addLog = useLogStore((s) => s.addLog);
  const [flagged, setFlagged] = useState(false);

  useEffect(() => {
    let current = true;
    setFlagged(false);
    api
      .hasAiControlMarkup(accountId, emailId)
      .then((result) => {
        if (current) setFlagged(result);
      })
      .catch((err) => {
        if (current) addLog('error', 'system', `${t('inbox:aiMarkup.checkFailed')}: ${errorText(err)}`);
      });
    return () => {
      current = false;
    };
  }, [accountId, emailId, addLog, t]);

  if (!flagged) return null;

  return (
    <div role="alert" className="mb-3 rounded border border-red-300 bg-red-50 px-3 py-2 text-xs text-red-900">
      <div className="font-medium">{t('inbox:aiMarkup.title')}</div>
      <div className="mt-0.5 opacity-90">{t('inbox:aiMarkup.detail')}</div>
    </div>
  );
}
