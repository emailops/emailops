import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { addImapAccount, type ImapAccountConfig, testImapConnection } from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Account } from '@/types';

interface Preset {
  label: string;
  imapHost: string;
  imapPort: number;
  smtpHost: string;
  smtpPort: number;
}

const PRESETS: Preset[] = [
  { label: 'Custom', imapHost: '', imapPort: 993, smtpHost: '', smtpPort: 587 },
  { label: 'iCloud', imapHost: 'imap.mail.me.com', imapPort: 993, smtpHost: 'smtp.mail.me.com', smtpPort: 587 },
  { label: 'Yahoo', imapHost: 'imap.mail.yahoo.com', imapPort: 993, smtpHost: 'smtp.mail.yahoo.com', smtpPort: 587 },
  {
    label: 'Outlook/Hotmail',
    imapHost: 'outlook.office365.com',
    imapPort: 993,
    smtpHost: 'smtp.office365.com',
    smtpPort: 587,
  },
  { label: 'Fastmail', imapHost: 'imap.fastmail.com', imapPort: 993, smtpHost: 'smtp.fastmail.com', smtpPort: 587 },
  { label: 'ProtonMail Bridge', imapHost: '127.0.0.1', imapPort: 1143, smtpHost: '127.0.0.1', smtpPort: 1025 },
];

type TestStatus = 'idle' | 'testing' | 'ok' | 'error';

interface Props {
  onSuccess: (account: Account) => void;
  onCancel: () => void;
}

export function AddImapAccountModal({ onSuccess, onCancel }: Props) {
  const { t } = useTranslation(['common', 'modal']);
  const [preset, setPreset] = useState<Preset>(PRESETS[0]);
  const [host, setHost] = useState('');
  const [port, setPort] = useState(993);
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [smtpHost, setSmtpHost] = useState('');
  const [smtpPort, setSmtpPort] = useState(587);
  const [displayName, setDisplayName] = useState('');

  const [testStatus, setTestStatus] = useState<TestStatus>('idle');
  const [testError, setTestError] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  function resetTest() {
    if (testStatus !== 'idle') {
      setTestStatus('idle');
      setTestError(null);
    }
  }

  function applyPreset(p: Preset) {
    setPreset(p);
    if (p.imapHost) setHost(p.imapHost);
    setPort(p.imapPort);
    if (p.smtpHost) setSmtpHost(p.smtpHost);
    setSmtpPort(p.smtpPort);
    resetTest();
  }

  function credentialField(setter: (v: string) => void) {
    return (e: React.ChangeEvent<HTMLInputElement>) => {
      setter(e.target.value);
      resetTest();
    };
  }

  function numberField(setter: (v: number) => void) {
    return (e: React.ChangeEvent<HTMLInputElement>) => {
      setter(Number(e.target.value));
      resetTest();
    };
  }

  async function handleTest() {
    setTestStatus('testing');
    setTestError(null);
    setSubmitError(null);
    try {
      await testImapConnection({
        host: host.trim(),
        port,
        username: username.trim(),
        password,
        smtpHost: smtpHost.trim(),
        smtpPort,
      });
      setTestStatus('ok');
    } catch (err) {
      setTestStatus('error');
      setTestError(errorText(err));
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (testStatus !== 'ok') return;
    setSubmitError(null);
    setSubmitting(true);
    try {
      const config: ImapAccountConfig = {
        host: host.trim(),
        port,
        username: username.trim(),
        password,
        smtpHost: smtpHost.trim(),
        smtpPort,
        displayName: displayName.trim() || undefined,
      };
      const account = await addImapAccount(config);
      onSuccess(account);
    } catch (err) {
      setSubmitError(errorText(err));
    } finally {
      setSubmitting(false);
    }
  }

  const canTest = host.trim() && username.trim() && password && smtpHost.trim() && testStatus !== 'testing';
  const canSubmit = testStatus === 'ok' && !submitting;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-neutral-900 border border-neutral-700 rounded-lg w-full max-w-md p-6 shadow-xl">
        <h2 className="text-lg font-semibold text-white mb-4">{t('modal:imapAccount.modalTitle')}</h2>

        {/* Provider presets */}
        <div className="flex flex-wrap gap-2 mb-4">
          {PRESETS.map((p) => (
            <button
              key={p.label}
              type="button"
              onClick={() => applyPreset(p)}
              className={`px-3 py-1 rounded text-xs font-medium border transition-colors ${
                preset.label === p.label
                  ? 'bg-blue-600 border-blue-500 text-white'
                  : 'bg-neutral-800 border-neutral-600 text-neutral-300 hover:bg-neutral-700'
              }`}
            >
              {p.label}
            </button>
          ))}
        </div>

        <form onSubmit={handleSubmit} className="space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <div className="col-span-2">
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.emailLabel')}</label>
              <input
                type="email"
                value={username}
                onChange={credentialField(setUsername)}
                required
                placeholder={'you@example.com'} // i18n-ignore: example email address, not user-facing copy
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white placeholder-neutral-500 focus:outline-none focus:border-blue-500"
              />
            </div>
            <div className="col-span-2">
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.passwordLabel')}</label>
              <input
                type="password"
                value={password}
                onChange={credentialField(setPassword)}
                required
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500"
              />
            </div>
            <div>
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.imapHost')}</label>
              <input
                type="text"
                value={host}
                onChange={credentialField(setHost)}
                required
                placeholder={'imap.example.com'} // i18n-ignore: example hostname, not user-facing copy
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white placeholder-neutral-500 focus:outline-none focus:border-blue-500"
              />
            </div>
            <div>
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.imapPort')}</label>
              <input
                type="number"
                value={port}
                onChange={numberField(setPort)}
                required
                min={1}
                max={65535}
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white focus:outline-none focus:border-blue-500"
              />
            </div>
            <div>
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.smtpHost')}</label>
              <input
                type="text"
                value={smtpHost}
                onChange={credentialField(setSmtpHost)}
                required
                placeholder={'smtp.example.com'} // i18n-ignore: example hostname, not user-facing copy
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white placeholder-neutral-500 focus:outline-none focus:border-blue-500"
              />
            </div>
            <div>
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.smtpPort')}</label>
              <input
                type="number"
                value={smtpPort}
                onChange={numberField(setSmtpPort)}
                required
                min={1}
                max={65535}
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white placeholder-neutral-500 focus:outline-none focus:border-blue-500"
              />
            </div>
            <div className="col-span-2">
              <label className="block text-xs text-neutral-400 mb-1">{t('modal:imapAccount.displayName')}</label>
              <input
                type="text"
                value={displayName}
                onChange={(e) => setDisplayName(e.target.value)}
                placeholder={t('modal:imapAccount.displayNamePlaceholder')}
                className="w-full bg-neutral-800 border border-neutral-600 rounded px-3 py-2 text-sm text-white placeholder-neutral-500 focus:outline-none focus:border-blue-500"
              />
            </div>
          </div>

          {/* Test connection */}
          <div className="flex items-center gap-3 pt-1">
            <button
              type="button"
              onClick={handleTest}
              disabled={!canTest}
              className="px-4 py-2 rounded bg-neutral-700 border border-neutral-600 text-neutral-200 text-sm hover:bg-neutral-600 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {testStatus === 'testing' ? 'Testing…' : 'Test Connection'}
            </button>
            {testStatus === 'ok' && (
              <span className="text-green-400 text-sm">{t('modal:imapAccount.testSuccess')}</span>
            )}
            {testStatus === 'error' && (
              <span className="text-red-400 text-sm">{t('modal:imapAccount.testFailedShort')}</span>
            )}
          </div>

          {testError && (
            <p className="text-red-400 text-xs bg-red-950/40 border border-red-900/50 rounded px-3 py-2">{testError}</p>
          )}

          {submitError && <p className="text-red-400 text-xs mt-1">{submitError}</p>}

          {testStatus !== 'ok' && testStatus !== 'error' && (
            <p className="text-neutral-500 text-xs">{t('modal:imapAccount.testHint')}</p>
          )}

          <div className="flex gap-3 pt-1">
            <button
              type="button"
              onClick={onCancel}
              disabled={submitting}
              className="flex-1 px-4 py-2 rounded bg-neutral-700 text-neutral-300 text-sm hover:bg-neutral-600 transition-colors disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!canSubmit}
              title={testStatus !== 'ok' ? 'Test the connection first' : undefined}
              className="flex-1 px-4 py-2 rounded bg-blue-600 text-white text-sm font-medium hover:bg-blue-500 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {submitting ? 'Adding…' : 'Add Account'}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
