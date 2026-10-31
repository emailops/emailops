import { type ReactNode, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { LockScreen } from './LockScreen';

/**
 * Shows the app only once it is known not to be locked. A check that fails
 * keeps the app hidden, as a password may well be set, and offers to try again.
 */
export function LockGate({ children }: { children: ReactNode }) {
  const { t } = useTranslation(['auth', 'common']);
  const [status, setStatus] = useState<'checking' | 'locked' | 'open' | 'failed'>('checking');

  const check = useCallback(() => {
    setStatus('checking');
    api
      .hasMainPassword()
      .then((has) => setStatus(has ? 'locked' : 'open'))
      .catch((err) => {
        useLogStore.getState().addLog('error', 'system', `Could not read the lock state: ${errorText(err)}`);
        setStatus('failed');
      });
  }, []);

  useEffect(check, [check]);

  if (status === 'checking') return null;
  if (status === 'locked') return <LockScreen onUnlock={() => setStatus('open')} />;
  if (status === 'failed') {
    return (
      <div className="min-h-screen flex flex-col items-center justify-center gap-3 bg-gray-50 px-4 text-center">
        <p className="text-sm text-gray-700">{t('auth:lock.checkFailed')}</p>
        <button
          type="button"
          onClick={check}
          className="px-4 py-2 text-sm font-medium text-white bg-primary-600 rounded-lg hover:bg-primary-700 transition-colors"
        >
          {t('common:actions.retry')}
        </button>
      </div>
    );
  }
  return children;
}
