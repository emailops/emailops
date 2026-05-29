import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { AddImapAccountModal } from '@/components/AddImapAccountModal';
import { AccountSettingsDialog } from '@/components/Sidebar/AccountSettingsDialog';
import { useAccounts } from '@/hooks/useAccounts';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';

export function StepAddAccount({ onBack, onComplete }: { onBack: () => void; onComplete: () => Promise<void> | void }) {
  const { t } = useTranslation(['auth', 'common']);
  const { addAccount, registerImapAccount, syncAccount, setAccountEnabled, removeAccount, refetch } = useAccounts();
  const addLog = useLogStore((s) => s.addLog);
  const [oauthInFlight, setOauthInFlight] = useState<'gmail' | 'outlook' | null>(null);
  const [showImap, setShowImap] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newAccount, setNewAccount] = useState<Account | null>(null);

  const startOAuth = async (provider: 'gmail' | 'outlook') => {
    if (oauthInFlight) return;
    setError(null);
    setOauthInFlight(provider);
    const label = provider === 'outlook' ? 'Outlook' : 'Gmail';
    try {
      addLog('info', 'account', `Adding ${label} account...`);
      const account = await addAccount(provider, null, { deferSetup: true });
      addLog('success', 'account', `Account added: ${account.email}`);
      setNewAccount(account);
    } catch (err) {
      const msg = errorText(err);
      addLog('error', 'account', `Failed to add account: ${msg}`);
      setError(msg);
    } finally {
      setOauthInFlight(null);
    }
  };

  const handleImapAdded = (account: Account) => {
    registerImapAccount(account, { deferSetup: true });
    addLog('success', 'account', `IMAP account added: ${account.email}`);
    setShowImap(false);
    setNewAccount(account);
  };

  const handleSettingsSaved = async () => {
    const id = newAccount?.id;
    setNewAccount(null);
    if (id) useAccountStore.getState().clearSetupPending(id);
    await refetch();
    if (id) {
      addLog('info', 'sync', 'Settings saved. Starting sync...');
      try {
        await syncAccount(id);
      } catch (e) {
        addLog('error', 'sync', `Sync failed: ${e}`);
      }
    }
    await onComplete();
  };

  const handleSettingsClose = async () => {
    const id = newAccount?.id;
    setNewAccount(null);
    if (id) useAccountStore.getState().clearSetupPending(id);
    await onComplete();
  };

  const handleSkip = async () => {
    addLog('info', 'system', 'Onboarding skipped — no account added');
    await onComplete();
  };

  return (
    <div className="space-y-5">
      <p className="text-sm text-gray-400">{t('auth:onboarding.addAccount.intro')}</p>

      <div className="space-y-2">
        <ProviderButton
          label={t('auth:onboarding.addAccount.gmailLabel')}
          description={
            oauthInFlight === 'gmail'
              ? t('auth:onboarding.addAccount.gmailWaiting')
              : t('auth:onboarding.addAccount.gmailIdle')
          }
          disabled={oauthInFlight !== null}
          onClick={() => void startOAuth('gmail')}
        />
        <ProviderButton
          label={t('auth:onboarding.addAccount.outlookLabel')}
          description={
            oauthInFlight === 'outlook'
              ? t('auth:onboarding.addAccount.outlookWaiting')
              : t('auth:onboarding.addAccount.outlookIdle')
          }
          disabled={oauthInFlight !== null}
          onClick={() => void startOAuth('outlook')}
        />
        <ProviderButton
          label={t('auth:onboarding.addAccount.imapLabel')}
          description={t('auth:onboarding.addAccount.imapDescription')}
          disabled={oauthInFlight !== null}
          onClick={() => setShowImap(true)}
        />
      </div>

      {error && <div className="p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>}

      <div className="flex items-center justify-between pt-2">
        <button
          onClick={onBack}
          className="px-4 py-2 text-sm text-gray-400 hover:text-gray-200 hover:bg-gray-800 rounded transition-colors"
        >
          {t('common:actions.back')}
        </button>
        <button
          onClick={() => void handleSkip()}
          className="text-sm text-gray-500 hover:text-gray-300 underline underline-offset-2"
        >
          {t('auth:onboarding.addAccount.skipForNow')}
        </button>
      </div>

      {showImap && (
        <AddImapAccountModal onSuccess={(account) => handleImapAdded(account)} onCancel={() => setShowImap(false)} />
      )}

      {newAccount && (
        <AccountSettingsDialog
          account={newAccount}
          onClose={() => void handleSettingsClose()}
          onSaved={() => void handleSettingsSaved()}
          onToggleEnabled={async (enabled) => {
            await setAccountEnabled(newAccount.id, enabled);
            await refetch();
          }}
          onDelete={async () => {
            const id = newAccount.id;
            addLog('info', 'account', 'Deleting account...');
            await removeAccount(id);
            setNewAccount(null);
            addLog('success', 'account', 'Account deleted.');
          }}
        />
      )}
    </div>
  );
}

function ProviderButton({
  label,
  description,
  onClick,
  disabled = false,
}: {
  label: string;
  description: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className="w-full text-left px-4 py-3 rounded-lg bg-[#27272a] border border-gray-700 hover:border-primary-500 hover:bg-[#2d2d2f] transition-colors disabled:opacity-60 disabled:cursor-not-allowed disabled:hover:border-gray-700 disabled:hover:bg-[#27272a]"
    >
      <div className="text-sm font-medium text-gray-100">{label}</div>
      <div className="text-xs text-gray-500 mt-0.5">{description}</div>
    </button>
  );
}
