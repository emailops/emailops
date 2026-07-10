import { useTranslation } from 'react-i18next';

interface ErrorBannerProps {
  message: string | null;
  /** Email of the account the error belongs to, when it is account-scoped.
   *  With multiple accounts configured the banner must say which one failed. */
  accountEmail?: string | null;
  onDismiss: () => void;
  onReauthenticate?: () => void;
}

export function ErrorBanner({ message, accountEmail, onDismiss, onReauthenticate }: ErrorBannerProps) {
  const { t } = useTranslation(['common']);
  if (!message) return null;

  // Check if this is an auth error that can be fixed by re-authenticating
  const isAuthError =
    message.toLowerCase().includes('session expired') ||
    message.toLowerCase().includes('token') ||
    message.toLowerCase().includes('auth');

  return (
    <div className="bg-red-50 border-l-4 border-red-500 p-4">
      <div className="flex items-start">
        <div className="flex-shrink-0">
          <svg className="h-5 w-5 text-red-400" viewBox="0 0 20 20" fill="currentColor">
            <path
              fillRule="evenodd"
              d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z"
              clipRule="evenodd"
            />
          </svg>
        </div>
        <div className="ml-3 flex-1">
          <p className="text-sm text-red-700">
            {accountEmail && <span className="font-semibold">{accountEmail}: </span>}
            {message}
          </p>
          {isAuthError && onReauthenticate && (
            <button
              onClick={onReauthenticate}
              className="mt-2 text-sm font-medium text-red-700 hover:text-red-800 underline"
            >
              {t('common:actions.signInAgain')}
            </button>
          )}
        </div>
        <div className="ml-auto pl-3">
          <button onClick={onDismiss} className="inline-flex text-red-400 hover:text-red-500 focus:outline-none">
            <span className="sr-only">{t('common:actions.dismiss')}</span>
            <svg className="h-5 w-5" viewBox="0 0 20 20" fill="currentColor">
              <path
                fillRule="evenodd"
                d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z"
                clipRule="evenodd"
              />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
