import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { Account } from '@/types';
import { SettingsPanel } from './SettingsPanel';

/** Backend preference keys (validated in `commands/preferences.rs`, read by
 *  `services::mail_notifications`). */
const PREF_ENABLED = 'notifications.new_mail.enabled';
const PREF_ACCOUNT_PREFIX = 'notifications.new_mail.account:';
const PREF_CONTENT = 'notifications.new_mail.content';
const PREF_ONLY_UNFOCUSED = 'notifications.new_mail.only_unfocused';

type ContentMode = 'preview' | 'hidden';

interface NotificationPrefs {
  enabled: boolean;
  onlyUnfocused: boolean;
  content: ContentMode;
  /** Account id → on. Missing = on (the default). */
  accounts: Record<string, boolean>;
}

const DEFAULTS: NotificationPrefs = { enabled: true, onlyUnfocused: true, content: 'preview', accounts: {} };

/** Every switch defaults to on; only an explicit "false" turns one off. */
const isOn = (raw: string | null) => raw !== 'false';

interface SwitchProps {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (next: boolean) => void;
}

function Switch({ label, checked, disabled, onChange }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors disabled:opacity-50 ${
        checked ? 'bg-primary-600' : 'bg-neutral-600'
      }`}
    >
      <span
        className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
          checked ? 'translate-x-5' : 'translate-x-1'
        }`}
      />
    </button>
  );
}

/**
 * Settings → Notifications: desktop notifications for new mail (and for
 * snoozed conversations coming back). Master switch, per-account switches,
 * what a notification may show, and whether to stay quiet while EmailOps has
 * the focus. Everything is a backend pref; the sync decides what notifies.
 */
export function NotificationsSettings({ accounts }: { accounts: Account[] }) {
  const { t } = useTranslation(['settings']);
  const addLog = useLogStore((s) => s.addLog);
  const [prefs, setPrefs] = useState<NotificationPrefs>(DEFAULTS);
  const [isLoaded, setIsLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const accountKey = accounts.map((a) => a.id).join(',');
  useEffect(() => {
    let cancelled = false;
    const ids = accountKey ? accountKey.split(',') : [];
    void (async () => {
      try {
        const [enabled, onlyUnfocused, content, ...perAccount] = await Promise.all([
          api.getPref(PREF_ENABLED),
          api.getPref(PREF_ONLY_UNFOCUSED),
          api.getPref(PREF_CONTENT),
          ...ids.map((id) => api.getPref(`${PREF_ACCOUNT_PREFIX}${id}`)),
        ]);
        if (cancelled) return;
        setPrefs({
          enabled: isOn(enabled),
          onlyUnfocused: isOn(onlyUnfocused),
          content: content === 'hidden' ? 'hidden' : 'preview',
          accounts: Object.fromEntries(ids.map((id, i) => [id, isOn(perAccount[i] ?? null)])),
        });
      } catch (e) {
        if (cancelled) return;
        setError(errorText(e));
        addLog('error', 'system', `Failed to load notification settings: ${errorText(e)}`);
      } finally {
        if (!cancelled) setIsLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accountKey, addLog]);

  /** Optimistic write: apply `next`, persist, roll back and report on failure. */
  const save = (next: NotificationPrefs, key: string, value: string) => {
    const previous = prefs;
    setPrefs(next);
    setError(null);
    api.setPref(key, value).catch((e) => {
      setPrefs(previous);
      setError(errorText(e));
      addLog('error', 'system', `Failed to save notification setting: ${errorText(e)}`);
    });
  };

  const masterOff = !prefs.enabled;
  const contentOptions: { value: ContentMode; label: string; desc: string }[] = [
    { value: 'preview', label: t('notifications.contentPreview'), desc: t('notifications.contentPreviewDesc') },
    { value: 'hidden', label: t('notifications.contentHidden'), desc: t('notifications.contentHiddenDesc') },
  ];

  return (
    <SettingsPanel
      header={
        error && (
          <div className="flex-shrink-0 mx-6 mt-4 border border-red-800 bg-red-950 text-red-200 text-sm rounded p-3">
            {error}
          </div>
        )
      }
    >
      <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <span className="text-sm font-medium text-gray-100">{t('notifications.enabledLabel')}</span>
            <p className="text-xs text-gray-400 mt-1">{t('notifications.enabledDesc')}</p>
          </div>
          <Switch
            label={t('notifications.enabledLabel')}
            checked={prefs.enabled}
            disabled={!isLoaded}
            onChange={(next) => save({ ...prefs, enabled: next }, PREF_ENABLED, String(next))}
          />
        </div>
      </section>

      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('notifications.accountsLabel')}</h3>
        <p className="text-xs text-gray-500 mb-2">{t('notifications.accountsDesc')}</p>
        {accounts.length === 0 ? (
          <p className="text-xs text-gray-400">{t('notifications.noAccounts')}</p>
        ) : (
          <div className="rounded-lg border border-gray-700 bg-[#1f1f20] divide-y divide-gray-700">
            {accounts.map((account) => {
              const on = prefs.accounts[account.id] ?? true;
              return (
                <div key={account.id} className="flex items-center justify-between gap-4 px-4 py-3">
                  <div className="min-w-0">
                    <span className="text-sm text-gray-100 block truncate">{account.email}</span>
                    <span className="text-xs text-gray-500 capitalize">{account.provider}</span>
                  </div>
                  <Switch
                    label={t('notifications.accountToggleAria', { email: account.email })}
                    checked={on}
                    disabled={!isLoaded || masterOff}
                    onChange={(next) =>
                      save(
                        { ...prefs, accounts: { ...prefs.accounts, [account.id]: next } },
                        `${PREF_ACCOUNT_PREFIX}${account.id}`,
                        String(next),
                      )
                    }
                  />
                </div>
              );
            })}
          </div>
        )}
      </section>

      <section>
        <h3 className="text-sm font-semibold text-gray-300 mb-1">{t('notifications.contentLabel')}</h3>
        <p className="text-xs text-gray-500 mb-2">{t('notifications.contentDesc')}</p>
        <div className="space-y-2">
          {contentOptions.map((option) => (
            <label key={option.value} className="flex items-start gap-2.5 cursor-pointer">
              <input
                type="radio"
                name="notification-content"
                value={option.value}
                checked={prefs.content === option.value}
                disabled={!isLoaded || masterOff}
                onChange={() => save({ ...prefs, content: option.value }, PREF_CONTENT, option.value)}
                className="mt-0.5 border-gray-600 bg-transparent text-primary-600 focus:ring-primary-600 focus:ring-offset-0"
              />
              <span className="min-w-0">
                <span className="text-sm text-gray-100 block">{option.label}</span>
                <span className="text-xs text-gray-400">{option.desc}</span>
              </span>
            </label>
          ))}
        </div>
      </section>

      <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <span className="text-sm font-medium text-gray-100">{t('notifications.onlyUnfocusedLabel')}</span>
            <p className="text-xs text-gray-400 mt-1">{t('notifications.onlyUnfocusedDesc')}</p>
          </div>
          <Switch
            label={t('notifications.onlyUnfocusedLabel')}
            checked={prefs.onlyUnfocused}
            disabled={!isLoaded || masterOff}
            onChange={(next) => save({ ...prefs, onlyUnfocused: next }, PREF_ONLY_UNFOCUSED, String(next))}
          />
        </div>
      </section>

      <p className="text-xs text-gray-500">{t('notifications.footnote')}</p>
    </SettingsPanel>
  );
}
