import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FALLBACK_LANGUAGE, isSupportedLanguage } from '@/i18n/resources';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { privacyPolicyUrl } from '@/lib/privacyPolicy';
import { useLogStore } from '@/stores/logStore';

// ── Small reusable toggle row ─────────────────────────────────────────────────

function ToggleRow({
  label,
  description,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="flex-1 min-w-0">
        <div className="text-sm font-medium text-gray-200">{label}</div>
        {description && <div className="text-xs text-gray-500 mt-0.5">{description}</div>}
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        disabled={disabled}
        className={`relative mt-0.5 inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2 focus:ring-offset-[#252526] disabled:opacity-50 ${
          checked ? 'bg-primary-600' : 'bg-gray-600'
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow ring-0 transition duration-200 ${
            checked ? 'translate-x-4' : 'translate-x-0'
          }`}
        />
      </button>
    </div>
  );
}

// ── Set/Change password modal ─────────────────────────────────────────────────

function PasswordDialog({
  mode,
  onClose,
  onSuccess,
}: {
  mode: 'set' | 'change' | 'remove';
  onClose: () => void;
  onSuccess: () => void;
}) {
  const { t } = useTranslation(['common', 'settings']);
  const [currentPassword, setCurrentPassword] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  const title =
    mode === 'set'
      ? t('settings:privacy.passwordDialog.setTitle')
      : mode === 'change'
        ? t('settings:privacy.passwordDialog.changeTitle')
        : t('settings:privacy.passwordDialog.removeTitle');

  const handleSubmit = useCallback(async () => {
    setError(null);
    if (mode !== 'remove') {
      if (newPassword.length < 6) {
        setError(t('settings:privacy.passwordDialog.minLengthError'));
        return;
      }
      if (newPassword !== confirmPassword) {
        setError(t('settings:privacy.passwordDialog.mismatchError'));
        return;
      }
    }

    setIsSaving(true);
    try {
      if (mode === 'remove') {
        await api.removeMainPassword(currentPassword);
      } else {
        await api.setMainPassword(mode === 'change' ? currentPassword : null, newPassword);
      }
      onSuccess();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setIsSaving(false);
    }
  }, [mode, currentPassword, newPassword, confirmPassword, onSuccess, t]);

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60">
      <div className="bg-[#2d2d2e] border border-gray-600 rounded-lg p-5 shadow-xl w-full max-w-sm mx-4">
        <h3 className="text-sm font-semibold text-gray-100 mb-4">{title}</h3>

        <div className="space-y-3">
          {(mode === 'change' || mode === 'remove') && (
            <div>
              <label className="block text-xs font-medium text-gray-400 mb-1">
                {t('settings:privacy.passwordDialog.currentPassword')}
              </label>
              <input
                type="password"
                // biome-ignore lint/a11y/noAutofocus: focus expected when password modal opens
                autoFocus
                value={currentPassword}
                onChange={(e) => setCurrentPassword(e.target.value)}
                className="w-full rounded-lg border border-gray-600 bg-[#1f1f20] px-3 py-2 text-sm text-gray-100 outline-none focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
              />
            </div>
          )}

          {mode !== 'remove' && (
            <>
              <div>
                <label className="block text-xs font-medium text-gray-400 mb-1">
                  {mode === 'set'
                    ? t('settings:privacy.passwordDialog.password')
                    : t('settings:privacy.passwordDialog.newPassword')}
                </label>
                <input
                  type="password"
                  // biome-ignore lint/a11y/noAutofocus: focus expected when password modal opens
                  autoFocus={mode === 'set'}
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  className="w-full rounded-lg border border-gray-600 bg-[#1f1f20] px-3 py-2 text-sm text-gray-100 outline-none focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-gray-400 mb-1">
                  {t('settings:privacy.passwordDialog.confirmPassword')}
                </label>
                <input
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  onKeyDown={(e) => e.key === 'Enter' && handleSubmit()}
                  className="w-full rounded-lg border border-gray-600 bg-[#1f1f20] px-3 py-2 text-sm text-gray-100 outline-none focus:border-primary-500 focus:ring-1 focus:ring-primary-500"
                />
              </div>
            </>
          )}

          {error && <p className="text-xs text-red-400">{error}</p>}
        </div>

        <div className="flex gap-2 justify-end mt-5">
          <button
            type="button"
            onClick={onClose}
            disabled={isSaving}
            className="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded transition-colors disabled:opacity-50"
          >
            {t('common:actions.cancel')}
          </button>
          <button
            type="button"
            onClick={handleSubmit}
            disabled={isSaving}
            className={`px-3 py-1.5 text-sm text-white rounded transition-colors disabled:opacity-50 ${
              mode === 'remove' ? 'bg-red-600 hover:bg-red-500' : 'bg-primary-600 hover:bg-primary-500'
            }`}
          >
            {isSaving
              ? t('settings:privacy.passwordDialog.saving')
              : mode === 'remove'
                ? t('settings:privacy.passwordDialog.removeButton')
                : t('common:actions.save')}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Main panel ────────────────────────────────────────────────────────────────

export function PrivacySettings() {
  const { t, i18n } = useTranslation(['common', 'settings']);
  const { addLog } = useLogStore();
  const [hasPassword, setHasPassword] = useState(false);
  const [allowRemoteContent, setAllowRemoteContent] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [dialog, setDialog] = useState<'set' | 'change' | 'remove' | null>(null);

  useEffect(() => {
    Promise.all([api.hasMainPassword(), api.getPref('privacy.allow_remote_content')]).then(([hasPw, remoteVal]) => {
      setHasPassword(hasPw);
      setAllowRemoteContent(remoteVal === 'true');
      setIsLoading(false);
    });
  }, []);

  const handleToggleRemoteContent = useCallback(async (value: boolean) => {
    setAllowRemoteContent(value);
    await api.setPref('privacy.allow_remote_content', value ? 'true' : 'false');
  }, []);

  const handlePasswordSuccess = useCallback(() => {
    setDialog(null);
    api.hasMainPassword().then(setHasPassword);
  }, []);

  const handleOpenPrivacyPolicy = useCallback(() => {
    const language = isSupportedLanguage(i18n.language) ? i18n.language : FALLBACK_LANGUAGE;
    void openExternal(privacyPolicyUrl(language)).catch((e) => {
      addLog('error', 'system', `Failed to open privacy policy: ${errorText(e)}`);
    });
  }, [i18n.language, addLog]);

  if (isLoading) {
    return (
      <div className="flex-1 flex items-center justify-center text-sm text-gray-500">
        {t('settings:privacy.loading')}
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-y-auto px-6 py-5 space-y-8">
      {/* Password ─────────────────────────────────────────────────────────── */}
      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:privacy.password')}</h3>
        <p className="text-xs text-gray-500 mb-3">{t('settings:privacy.passwordHelp')}</p>

        <div className="rounded-lg border border-gray-700 bg-[#1f1f20] divide-y divide-gray-700">
          <div className="px-4 py-3">
            <ToggleRow
              label={t('settings:privacy.usePassword')}
              description={hasPassword ? t('settings:privacy.lockedDesc') : t('settings:privacy.noPasswordDesc')}
              checked={hasPassword}
              onChange={(v) => setDialog(v ? 'set' : 'remove')}
            />
          </div>

          {hasPassword && (
            <div className="px-4 py-3">
              <button
                type="button"
                onClick={() => setDialog('change')}
                className="text-sm text-primary-400 hover:text-primary-300 transition-colors"
              >
                {t('settings:privacy.changePassword')}
              </button>
            </div>
          )}
        </div>
      </section>

      {/* Privacy ─────────────────────────────────────────────────────────── */}
      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('settings:privacy.title')}</h3>

        <div className="mt-3">
          <h4 className="text-xs font-semibold text-gray-400 uppercase tracking-wide mb-2">
            {t('settings:privacy.emailContent')}
          </h4>
          <div className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
            <ToggleRow
              label={t('settings:privacy.allowRemoteToggle')}
              description={t('settings:privacy.allowRemoteToggleDesc')}
              checked={allowRemoteContent}
              onChange={handleToggleRemoteContent}
            />
          </div>
        </div>

        <div className="mt-3">
          <button
            type="button"
            onClick={handleOpenPrivacyPolicy}
            className="text-sm text-primary-400 hover:text-primary-300 transition-colors"
          >
            {t('settings:privacy.privacyPolicyLink')} ↗
          </button>
        </div>
      </section>

      {dialog && <PasswordDialog mode={dialog} onClose={() => setDialog(null)} onSuccess={handlePasswordSuccess} />}
    </div>
  );
}
