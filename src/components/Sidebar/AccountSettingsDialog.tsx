import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import type { Account } from '@/types';

type SyncPreset = '7d' | '30d' | '90d' | '365d' | 'all' | 'custom';

const SYNC_PRESETS: { id: SyncPreset; label: string; description: string }[] = [
  { id: '7d', label: 'Last 7 days', description: 'Fastest setup, recent email only.' },
  { id: '30d', label: 'Last 30 days', description: 'Good default for lightweight use.' },
  { id: '90d', label: 'Last 90 days', description: 'Useful if you need recent project history.' },
  { id: '365d', label: 'Last year', description: 'Broader history without syncing everything.' },
  { id: 'all', label: 'All mail', description: 'Import everything available.' },
  { id: 'custom', label: 'Custom date', description: 'Choose the exact start date.' },
];

const GMAIL_CATEGORIES: { id: string; label: string; description: string }[] = [
  { id: 'primary', label: 'Primary', description: 'Direct messages and important emails' },
  { id: 'social', label: 'Social', description: 'Social networks, gaming, dating' },
  { id: 'updates', label: 'Updates', description: 'Notifications, receipts, statements' },
  { id: 'forums', label: 'Forums', description: 'Mailing lists, digests, group messages' },
  { id: 'promotions', label: 'Promotions', description: 'Deals, offers, marketing emails' },
];

function formatDate(date: Date): string {
  const y = date.getFullYear();
  const m = `${date.getMonth() + 1}`.padStart(2, '0');
  const d = `${date.getDate()}`.padStart(2, '0');
  return `${y}-${m}-${d}`;
}

function presetFromTimestamp(ts: number | null): { preset: SyncPreset; customDate: string } {
  if (ts == null) return { preset: 'all', customDate: formatDate(new Date()) };
  const now = new Date();
  now.setHours(0, 0, 0, 0);
  const sel = new Date(ts * 1000);
  sel.setHours(0, 0, 0, 0);
  const days = Math.round((now.getTime() - sel.getTime()) / 86400000);
  const preset = days === 7 ? '7d' : days === 30 ? '30d' : days === 90 ? '90d' : days === 365 ? '365d' : 'custom';
  return { preset, customDate: formatDate(sel) };
}

function presetToTimestamp(preset: SyncPreset, customDate: string): number | null {
  if (preset === 'all') return null;
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  if (preset === '7d') {
    d.setDate(d.getDate() - 7);
    return Math.floor(d.getTime() / 1000);
  }
  if (preset === '30d') {
    d.setDate(d.getDate() - 30);
    return Math.floor(d.getTime() / 1000);
  }
  if (preset === '90d') {
    d.setDate(d.getDate() - 90);
    return Math.floor(d.getTime() / 1000);
  }
  if (preset === '365d') {
    d.setDate(d.getDate() - 365);
    return Math.floor(d.getTime() / 1000);
  }
  const sel = new Date(`${customDate}T00:00:00`);
  return Number.isNaN(sel.getTime()) ? null : Math.floor(sel.getTime() / 1000);
}

interface AccountSettingsDialogProps {
  account: Account;
  onClose: () => void;
  onSaved: () => void;
  onToggleEnabled: (enabled: boolean) => Promise<void>;
  onDelete: () => Promise<void>;
}

export function AccountSettingsDialog({
  account,
  onClose,
  onSaved,
  onToggleEnabled,
  onDelete,
}: AccountSettingsDialogProps) {
  const { t } = useTranslation(['modal']);
  const isGmail = account.provider === 'gmail';
  const isImap = account.provider === 'imap';

  const initialSync = useMemo(() => presetFromTimestamp(account.syncFromTimestamp), [account.syncFromTimestamp]);
  const [preset, setPreset] = useState<SyncPreset>(initialSync.preset);
  const [customDate, setCustomDate] = useState(initialSync.customDate);
  const [selectedCategories, setSelectedCategories] = useState<Set<string>>(new Set(['primary', 'updates']));
  const [autoDownloadCategories, setAutoDownloadCategories] = useState<Set<string>>(new Set());
  const [isLoading, setIsLoading] = useState(isGmail || isImap);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [syncEnabled, setSyncEnabled] = useState(account.enabled);
  const [isTogglingEnabled, setIsTogglingEnabled] = useState(false);
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false);
  const [deleteConfirmEmail, setDeleteConfirmEmail] = useState('');
  const [isDeleting, setIsDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);

  // IMAP credential fields
  const [imapHost, setImapHost] = useState('');
  const [imapPort, setImapPort] = useState<number>(993);
  const [imapUsername, setImapUsername] = useState('');
  const [imapPassword, setImapPassword] = useState('');
  const [smtpHost, setSmtpHost] = useState('');
  const [smtpPort, setSmtpPort] = useState<number>(465);
  const [showImapPassword, setShowImapPassword] = useState(false);
  const [imapDirty, setImapDirty] = useState(false);
  const [imapTesting, setImapTesting] = useState(false);
  const [imapTestStatus, setImapTestStatus] = useState<'idle' | 'ok' | 'fail'>('idle');
  const [imapTestMessage, setImapTestMessage] = useState<string | null>(null);
  const [imapLoadError, setImapLoadError] = useState<string | null>(null);

  useEffect(() => {
    if (isGmail) {
      api
        .getAccountSettings(account.id)
        .then((s) => {
          setSelectedCategories(new Set(s.gmailCategories));
          setAutoDownloadCategories(new Set(s.autoDownloadAttachmentCategories ?? []));
        })
        .catch(() => {
          /* use defaults */
        })
        .finally(() => setIsLoading(false));
    } else if (isImap) {
      // getImapSettings always returns the server fields from the DB mirror,
      // even when the keychain entry is gone. `hasPassword: false` means the
      // password specifically is missing — the dialog pre-fills everything
      // else and asks the user to retype the password (the common re-auth
      // case after a provider rotated their app password). Only the catch
      // path runs when something is genuinely broken (unknown account, DB
      // failure, etc.) in which case we fall back to pre-filling the
      // username from the account email.
      api
        .getImapSettings(account.id)
        .then((s) => {
          setImapHost(s.host);
          setImapPort(s.port);
          setImapUsername(s.username || account.email);
          setImapPassword(s.password);
          setSmtpHost(s.smtpHost);
          setSmtpPort(s.smtpPort);
          setImapDirty(false);
          setImapLoadError(
            s.hasPassword
              ? null
              : 'Saved password is missing from the keychain. Re-enter your password to reconnect — the server settings below have been kept.',
          );
        })
        .catch((e) => {
          setImapLoadError(errorText(e));
          setImapUsername((prev) => prev || account.email);
        })
        .finally(() => setIsLoading(false));
    }
  }, [account.id, account.email, isGmail, isImap]);

  const markImapDirty = useCallback(() => {
    setImapDirty(true);
    setImapTestStatus('idle');
    setImapTestMessage(null);
  }, []);

  const handleTestImap = useCallback(async () => {
    setImapTesting(true);
    setImapTestStatus('idle');
    setImapTestMessage(null);
    try {
      await api.testImapConnection({
        host: imapHost,
        port: imapPort,
        username: imapUsername,
        password: imapPassword,
        smtpHost,
        smtpPort,
      });
      setImapTestStatus('ok');
      setImapTestMessage('Connection succeeded.');
    } catch (e) {
      setImapTestStatus('fail');
      setImapTestMessage(errorText(e));
    } finally {
      setImapTesting(false);
    }
  }, [imapHost, imapPort, imapUsername, imapPassword, smtpHost, smtpPort]);

  const toggleCategory = useCallback((id: string) => {
    setSelectedCategories((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleAutoDownload = useCallback((id: string) => {
    setAutoDownloadCategories((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleToggleEnabled = useCallback(async () => {
    const next = !syncEnabled;
    setIsTogglingEnabled(true);
    try {
      await onToggleEnabled(next);
      setSyncEnabled(next);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsTogglingEnabled(false);
    }
  }, [syncEnabled, onToggleEnabled]);

  const handleDelete = useCallback(async () => {
    setIsDeleting(true);
    setDeleteError(null);
    try {
      await onDelete();
    } catch (e) {
      setDeleteError(errorText(e));
      setIsDeleting(false);
    }
  }, [onDelete]);

  const syncFromTimestamp = useMemo(() => presetToTimestamp(preset, customDate), [preset, customDate]);
  const isCustomInvalid = preset === 'custom' && syncFromTimestamp === null;

  const handleSave = useCallback(async () => {
    setIsSaving(true);
    setError(null);
    try {
      if (isGmail) {
        await api.setAccountSettings(account.id, {
          gmailCategories: Array.from(selectedCategories),
          autoDownloadAttachmentCategories: Array.from(autoDownloadCategories),
        });
      }
      if (isImap && imapDirty) {
        await api.updateImapCredentials(account.id, {
          host: imapHost,
          port: imapPort,
          username: imapUsername,
          password: imapPassword,
          smtpHost,
          smtpPort,
        });
        setImapDirty(false);
      }
      await api.updateAccountSyncFrom(account.id, syncFromTimestamp);
      onSaved();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsSaving(false);
    }
  }, [
    account.id,
    isGmail,
    isImap,
    selectedCategories,
    autoDownloadCategories,
    imapDirty,
    imapHost,
    imapPort,
    imapUsername,
    imapPassword,
    smtpHost,
    smtpPort,
    syncFromTimestamp,
    onSaved,
  ]);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4">
      <div className="w-full max-w-lg rounded-xl bg-[#1f1f20] shadow-2xl max-h-[90vh] flex flex-col">
        <div className="border-b border-gray-700 px-6 py-4 flex-shrink-0">
          <h2 className="text-lg font-semibold text-gray-100">{t('modal:accountSettings.modalTitle')}</h2>
          <p className="mt-0.5 text-sm text-gray-500">{account.email}</p>
        </div>

        <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
          {isLoading ? (
            <div className="text-sm text-gray-500">{t('modal:accountSettings.loading')}</div>
          ) : (
            <>
              <section>
                <h3 className="text-sm font-semibold text-gray-100 mb-3">{t('modal:accountSettings.syncHeading')}</h3>
                <div className="flex items-start gap-3 rounded-lg border border-gray-700 p-3">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={syncEnabled}
                    onClick={handleToggleEnabled}
                    disabled={isTogglingEnabled}
                    className={`relative mt-0.5 inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2 disabled:opacity-50 ${
                      syncEnabled ? 'bg-primary-600' : 'bg-gray-600'
                    }`}
                  >
                    <span
                      className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                        syncEnabled ? 'translate-x-4' : 'translate-x-0'
                      }`}
                    />
                  </button>
                  <div>
                    <div className="text-sm font-medium text-gray-100">
                      {syncEnabled ? 'Syncing enabled' : 'Sync paused'}
                    </div>
                    <div className="text-xs text-gray-500">
                      {syncEnabled
                        ? 'New emails will be downloaded on each sync.'
                        : 'Existing emails remain accessible but no new emails will be synced until re-enabled.'}
                    </div>
                  </div>
                </div>
              </section>

              {isGmail && (
                <section>
                  <h3 className="text-sm font-semibold text-gray-100 mb-3">
                    {t('modal:accountSettings.categoriesHeading')}
                  </h3>
                  <p className="text-xs text-gray-500 mb-3">{t('modal:accountSettings.categoriesHint')}</p>
                  <div className="space-y-2">
                    {GMAIL_CATEGORIES.map((cat) => (
                      <label
                        key={cat.id}
                        className="flex items-start gap-3 cursor-pointer rounded-lg border border-gray-700 p-3 hover:border-gray-500 transition-colors"
                      >
                        <input
                          type="checkbox"
                          checked={selectedCategories.has(cat.id)}
                          onChange={() => toggleCategory(cat.id)}
                          className="mt-0.5 h-4 w-4 rounded border-gray-700 text-primary-600 focus:ring-primary-500"
                        />
                        <div>
                          <div className="text-sm font-medium text-gray-100">{cat.label}</div>
                          <div className="text-xs text-gray-500">{cat.description}</div>
                        </div>
                      </label>
                    ))}
                  </div>
                </section>
              )}

              {isGmail && (
                <section>
                  <h3 className="text-sm font-semibold text-gray-100 mb-1">
                    {t('modal:accountSettings.autoDownloadHeading')}
                  </h3>
                  <p className="text-xs text-gray-500 mb-3">
                    By default attachments are only fetched when you click them. Enable auto-download for categories
                    where you want files saved to disk during sync (uses more storage). Emails matching an attachment
                    rule always download regardless of this setting.
                  </p>
                  <div className="space-y-2">
                    {GMAIL_CATEGORIES.filter((cat) => selectedCategories.has(cat.id)).map((cat) => (
                      <label
                        key={cat.id}
                        className="flex items-start gap-3 cursor-pointer rounded-lg border border-gray-700 p-3 hover:border-gray-500 transition-colors"
                      >
                        <input
                          type="checkbox"
                          checked={autoDownloadCategories.has(cat.id)}
                          onChange={() => toggleAutoDownload(cat.id)}
                          className="mt-0.5 h-4 w-4 rounded border-gray-700 text-primary-600 focus:ring-primary-500"
                        />
                        <div>
                          <div className="text-sm font-medium text-gray-100">{cat.label}</div>
                          <div className="text-xs text-gray-500">{cat.description}</div>
                        </div>
                      </label>
                    ))}
                    {selectedCategories.size === 0 && (
                      <p className="text-xs text-gray-400">{t('modal:accountSettings.autoDownloadEmptyCategories')}</p>
                    )}
                  </div>
                </section>
              )}

              {isImap && (
                <section>
                  <h3 className="text-sm font-semibold text-gray-100 mb-1">{t('modal:accountSettings.imapHeading')}</h3>
                  <p className="text-xs text-gray-500 mb-3">{t('modal:accountSettings.imapHint')}</p>
                  {imapLoadError && (
                    <div className="mb-3 rounded-lg border border-red-800 bg-red-900/20 px-3 py-2 text-sm text-red-300">
                      {imapLoadError}. Enter your credentials below to sign in again.
                    </div>
                  )}
                  <div className="space-y-3">
                    <div className="grid grid-cols-[1fr_6rem] gap-3">
                      <div>
                        <label className="block text-xs font-medium text-gray-300" htmlFor="imap-host">
                          {t('modal:accountSettings.imapServer')}
                        </label>
                        <input
                          id="imap-host"
                          type="text"
                          value={imapHost}
                          onChange={(e) => {
                            setImapHost(e.target.value);
                            markImapDirty();
                          }}
                          placeholder={'imap.example.com'} // i18n-ignore: example hostname, not user-facing copy
                          className="mt-1 w-full rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40"
                        />
                      </div>
                      <div>
                        <label className="block text-xs font-medium text-gray-300" htmlFor="imap-port">
                          Port
                        </label>
                        <input
                          id="imap-port"
                          type="number"
                          min={1}
                          max={65535}
                          value={imapPort}
                          onChange={(e) => {
                            setImapPort(Number(e.target.value) || 0);
                            markImapDirty();
                          }}
                          className="mt-1 w-full rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40"
                        />
                      </div>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-gray-300" htmlFor="imap-username">
                        Username
                      </label>
                      <input
                        id="imap-username"
                        type="text"
                        autoComplete="off"
                        value={imapUsername}
                        onChange={(e) => {
                          setImapUsername(e.target.value);
                          markImapDirty();
                        }}
                        className="mt-1 w-full rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-gray-300" htmlFor="imap-password">
                        Password
                      </label>
                      <div className="mt-1 flex gap-2">
                        <input
                          id="imap-password"
                          type={showImapPassword ? 'text' : 'password'}
                          autoComplete="new-password"
                          value={imapPassword}
                          onChange={(e) => {
                            setImapPassword(e.target.value);
                            markImapDirty();
                          }}
                          className="flex-1 rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40"
                        />
                        <button
                          type="button"
                          onClick={() => setShowImapPassword((s) => !s)}
                          className="rounded-lg border border-gray-700 px-3 text-xs text-gray-400 hover:bg-gray-800"
                        >
                          {showImapPassword ? 'Hide' : 'Show'}
                        </button>
                      </div>
                    </div>

                    <div className="grid grid-cols-[1fr_6rem] gap-3">
                      <div>
                        <label className="block text-xs font-medium text-gray-300" htmlFor="smtp-host">
                          {t('modal:accountSettings.smtpServer')}
                        </label>
                        <input
                          id="smtp-host"
                          type="text"
                          value={smtpHost}
                          onChange={(e) => {
                            setSmtpHost(e.target.value);
                            markImapDirty();
                          }}
                          placeholder={'smtp.example.com'} // i18n-ignore: example hostname, not user-facing copy
                          className="mt-1 w-full rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40"
                        />
                      </div>
                      <div>
                        <label className="block text-xs font-medium text-gray-300" htmlFor="smtp-port">
                          Port
                        </label>
                        <input
                          id="smtp-port"
                          type="number"
                          min={1}
                          max={65535}
                          value={smtpPort}
                          onChange={(e) => {
                            setSmtpPort(Number(e.target.value) || 0);
                            markImapDirty();
                          }}
                          className="mt-1 w-full rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40"
                        />
                      </div>
                    </div>

                    <div className="flex items-center gap-3">
                      <button
                        type="button"
                        onClick={handleTestImap}
                        disabled={imapTesting || !imapHost || !imapUsername || !imapPassword || !smtpHost}
                        className="rounded-lg border border-gray-700 px-3 py-2 text-xs font-medium text-gray-300 hover:bg-gray-800 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {imapTesting ? 'Testing...' : 'Test connection'}
                      </button>
                      {imapTestStatus === 'ok' && <span className="text-xs text-green-400">{imapTestMessage}</span>}
                      {imapTestStatus === 'fail' && <span className="text-xs text-red-300">{imapTestMessage}</span>}
                    </div>
                  </div>
                </section>
              )}

              <section>
                <h3 className="text-sm font-semibold text-gray-100 mb-3">
                  {t('modal:accountSettings.syncPeriodHeading')}
                </h3>
                <p className="text-xs text-gray-500 mb-3">{t('modal:accountSettings.syncPeriodHint')}</p>
                <div className="space-y-2">
                  {SYNC_PRESETS.map((opt) => (
                    <label
                      key={opt.id}
                      className={`block cursor-pointer rounded-lg border p-3 transition-colors ${
                        preset === opt.id
                          ? 'border-primary-500 bg-primary-900/20'
                          : 'border-gray-700 hover:border-gray-500'
                      }`}
                    >
                      <div className="flex items-start gap-3">
                        <input
                          type="radio"
                          name="syncPreset"
                          value={opt.id}
                          checked={preset === opt.id}
                          onChange={() => setPreset(opt.id)}
                          className="mt-1 h-4 w-4 border-gray-700 text-primary-600 focus:ring-primary-500"
                        />
                        <div>
                          <div className="text-sm font-medium text-gray-100">{opt.label}</div>
                          <div className="text-xs text-gray-500">{opt.description}</div>
                        </div>
                      </div>
                    </label>
                  ))}
                </div>

                {preset === 'custom' && (
                  <div className="mt-2 rounded-lg border border-gray-700 bg-[#27272a] p-3">
                    <label className="block text-sm font-medium text-gray-300" htmlFor="sync-from-date">
                      {t('modal:accountSettings.syncSince')}
                    </label>
                    <input
                      id="sync-from-date"
                      type="date"
                      value={customDate}
                      max={formatDate(new Date())}
                      onChange={(e) => setCustomDate(e.target.value)}
                      className="mt-2 w-full rounded-lg border border-gray-700 bg-[#27272a] text-gray-100 placeholder:text-gray-500 px-3 py-2 text-sm outline-none focus:border-primary-500 focus:ring-2 focus:ring-primary-900/40 [color-scheme:dark]"
                    />
                    {isCustomInvalid && <p className="mt-2 text-xs text-red-400">Pick a valid date to continue.</p>}
                  </div>
                )}
              </section>

              <section className="border-t border-red-900/40 pt-5">
                <h3 className="text-sm font-semibold text-red-300 mb-3">{t('modal:accountSettings.dangerHeading')}</h3>
                {!showDeleteConfirm ? (
                  <button
                    type="button"
                    onClick={() => setShowDeleteConfirm(true)}
                    className="rounded-lg border border-red-800 px-3 py-2 text-sm font-medium text-red-300 hover:bg-red-900/20 transition-colors"
                  >
                    {t('modal:accountSettings.deleteAccount')}
                  </button>
                ) : (
                  <div className="rounded-lg border border-red-800 bg-red-900/20 p-4 space-y-3">
                    <p className="text-sm font-medium text-red-200">{t('modal:accountSettings.dangerWarning')}</p>
                    <p className="text-xs text-red-300">{t('modal:accountSettings.deleteWarning')}</p>
                    <p className="font-mono text-xs text-red-400 bg-red-900/30 rounded px-2 py-1 select-all">
                      {account.email}
                    </p>
                    <input
                      type="email"
                      value={deleteConfirmEmail}
                      onChange={(e) => setDeleteConfirmEmail(e.target.value)}
                      placeholder={account.email}
                      className="w-full rounded-lg border border-red-800 bg-red-900/10 text-gray-100 placeholder:text-red-400/60 px-3 py-2 text-sm outline-none focus:border-red-500 focus:ring-2 focus:ring-red-900/40"
                    />
                    {deleteError && <p className="text-xs text-red-300">{deleteError}</p>}
                    <div className="flex gap-2">
                      <button
                        type="button"
                        onClick={() => {
                          setShowDeleteConfirm(false);
                          setDeleteConfirmEmail('');
                          setDeleteError(null);
                        }}
                        disabled={isDeleting}
                        className="rounded-lg border border-gray-700 px-3 py-2 text-sm font-medium text-gray-300 hover:bg-gray-800 disabled:opacity-50"
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        onClick={handleDelete}
                        disabled={isDeleting || deleteConfirmEmail !== account.email}
                        className="rounded-lg bg-red-600 px-3 py-2 text-sm font-medium text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-50"
                      >
                        {isDeleting ? 'Deleting...' : 'Delete account'}
                      </button>
                    </div>
                  </div>
                )}
              </section>

              {error && (
                <div className="rounded-lg border border-red-800 bg-red-900/20 px-3 py-2 text-sm text-red-300">
                  {error}
                </div>
              )}
            </>
          )}
        </div>

        <div className="flex items-center justify-end gap-3 border-t border-gray-700 px-6 py-4 flex-shrink-0">
          <button
            type="button"
            onClick={onClose}
            disabled={isSaving || isDeleting}
            className="rounded-lg px-4 py-2 text-sm font-medium text-gray-400 hover:bg-gray-800 disabled:opacity-50"
          >
            Cancel
          </button>
          {!showDeleteConfirm && (
            <button
              type="button"
              onClick={handleSave}
              disabled={
                isSaving ||
                isLoading ||
                isCustomInvalid ||
                (isImap &&
                  imapDirty &&
                  (!imapHost || !imapUsername || !imapPassword || !smtpHost || !imapPort || !smtpPort))
              }
              className="rounded-lg bg-primary-600 px-4 py-2 text-sm font-medium text-white hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {isSaving ? 'Saving...' : 'Save & Sync'}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
