import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';

interface LockScreenProps {
  onUnlock: () => void;
}

export function LockScreen({ onUnlock }: LockScreenProps) {
  const { t } = useTranslation(['auth']);
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isVerifying, setIsVerifying] = useState(false);

  const handleUnlock = useCallback(async () => {
    if (!password) return;
    setIsVerifying(true);
    setError(null);
    try {
      const ok = await api.verifyMainPassword(password);
      if (ok) {
        onUnlock();
      } else {
        setError(t('auth:lock.incorrectPassword'));
        setPassword('');
      }
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsVerifying(false);
    }
  }, [password, onUnlock, t]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleUnlock();
  };

  return (
    <div className="fixed inset-0 z-[9999] flex items-center justify-center bg-[#1a1a1b]">
      <div className="w-full max-w-sm px-4">
        <div className="text-center mb-8">
          <div className="inline-flex items-center justify-center w-14 h-14 rounded-full bg-primary-900/40 border border-primary-700 mb-4">
            <svg className="w-7 h-7 text-primary-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={1.5}
                d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z"
              />
            </svg>
          </div>
          <h1 className="text-xl font-semibold text-gray-100">{t('auth:lock.title')}</h1>
          <p className="text-sm text-gray-500 mt-1">{t('auth:lock.subtitle')}</p>
        </div>

        <div className="space-y-3">
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t('auth:lock.mainPasswordPlaceholder')}
            // biome-ignore lint/a11y/noAutofocus: lock screen is the only interactive element on mount
            autoFocus
            className="w-full rounded-lg border border-gray-700 bg-[#2d2d2e] px-4 py-3 text-sm text-gray-100 placeholder-gray-500 outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-500/20"
          />

          {error && <p className="text-xs text-red-400 text-center">{error}</p>}

          <button
            type="button"
            onClick={handleUnlock}
            disabled={!password || isVerifying}
            className="w-full rounded-lg bg-primary-600 py-3 text-sm font-medium text-white hover:bg-primary-500 disabled:cursor-not-allowed disabled:opacity-50 transition-colors"
          >
            {isVerifying ? t('auth:lock.verifying') : t('auth:lock.unlock')}
          </button>
        </div>
      </div>
    </div>
  );
}
