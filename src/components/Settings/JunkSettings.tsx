import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ToggleSwitch } from '@/components/common/ToggleSwitch';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useLogStore } from '@/stores/logStore';
import type { JunkConfig, JunkFlaggedAction, JunkStats } from '@/types';

/** Axes with a trained model. Typed so a missing translation fails the build. */
const MODEL_AXES = ['spam', 'graymail'] as const;
type ModelAxis = (typeof MODEL_AXES)[number];

function isModelAxis(axis: string): axis is ModelAxis {
  return (MODEL_AXES as readonly string[]).includes(axis);
}

interface JunkSettingsProps {
  activeAccountId: string | null;
}

const DEFAULT_CONFIG: JunkConfig = {
  enabled: true,
  phishingEnabled: false,
  flaggedAction: 'dim',
};

const ACTIONS: JunkFlaggedAction[] = ['dim', 'hide'];

export function JunkSettings({ activeAccountId }: JunkSettingsProps) {
  const { t } = useTranslation(['settings', 'common']);
  const addLog = useLogStore((s) => s.addLog);
  const accounts = useAccountStore((s) => s.accounts);
  const [config, setConfig] = useState<JunkConfig>(DEFAULT_CONFIG);
  const [stats, setStats] = useState<JunkStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [backfilling, setBackfilling] = useState(false);

  // The counts below belong to one mailbox, not to the install. Naming it is not
  // decoration: "203 messages scored" is meaningless — and quietly alarming —
  // when the reader has several accounts connected and cannot tell which one the
  // numbers describe.
  const activeAccountEmail = accounts.find((a) => a.id === activeAccountId)?.email ?? null;

  useEffect(() => {
    api
      .getJunkConfig()
      .then(setConfig)
      .catch((e) => setError(errorText(e)));
  }, []);

  const refreshStats = useCallback(() => {
    if (!activeAccountId) {
      setStats(null);
      return;
    }
    api
      .getJunkStats(activeAccountId)
      .then(setStats)
      .catch(() => setStats(null));
  }, [activeAccountId]);

  useEffect(refreshStats, [refreshStats]);

  const save = useCallback(
    async (next: JunkConfig) => {
      const previous = config;
      setConfig(next);
      try {
        await api.setJunkConfig(next);
      } catch (e) {
        // Roll back rather than leave the switch showing a state the backend
        // never accepted.
        setConfig(previous);
        setError(errorText(e));
      }
    },
    [config],
  );

  const handleBackfill = useCallback(async () => {
    if (!activeAccountId) return;
    setBackfilling(true);
    try {
      await api.backfillJunkScores(activeAccountId);
      addLog('info', 'system', t('settings:junk.backfillStarted'));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBackfilling(false);
    }
  }, [activeAccountId, addLog, t]);

  return (
    <div className="space-y-5">
      {/* Errors sit above everything, never at the bottom of a scroll container. */}
      {error && (
        <div className="rounded-lg border border-red-800 bg-red-900/30 px-3 py-2 text-sm text-red-200">{error}</div>
      )}

      <ToggleSwitch
        checked={config.enabled}
        onChange={(v) => void save({ ...config, enabled: v })}
        label={<span className="text-sm font-medium text-gray-100">{t('settings:junk.enabled')}</span>}
        description={<span className="text-xs text-gray-500">{t('settings:junk.enabledDesc')}</span>}
      />

      <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
        <h3 className="text-sm font-semibold text-gray-300 mb-2">{t('settings:junk.flaggedActionTitle')}</h3>
        <div className="space-y-2">
          {ACTIONS.map((action) => (
            <label key={action} className="flex items-start gap-2 cursor-pointer">
              <input
                type="radio"
                name="junk-flagged-action"
                className="mt-1 accent-primary-600"
                checked={config.flaggedAction === action}
                onChange={() => void save({ ...config, flaggedAction: action })}
                disabled={!config.enabled}
              />
              <span className="text-sm">
                <span className="text-gray-200">{t(`settings:junk.action.${action}` as const)}</span>
                <span className="block text-xs text-gray-500">{t(`settings:junk.actionDesc.${action}` as const)}</span>
              </span>
            </label>
          ))}
        </div>
        <p className="mt-3 text-xs text-gray-500">{t('settings:junk.neverMoves')}</p>
      </section>

      <ToggleSwitch
        checked={config.phishingEnabled}
        onChange={(v) => void save({ ...config, phishingEnabled: v })}
        label={<span className="text-sm font-medium text-gray-100">{t('settings:junk.phishing')}</span>}
        description={<span className="text-xs text-gray-500">{t('settings:junk.phishingDesc')}</span>}
      />

      {stats && (
        <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
          <h3 className="text-sm font-semibold text-gray-300">{t('settings:junk.statusTitle')}</h3>
          {activeAccountEmail && <p className="mb-2 text-xs text-gray-500">{activeAccountEmail}</p>}

          <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-xs">
            <dt className="text-gray-400">{t('settings:junk.scored')}</dt>
            <dd className="text-right tabular-nums text-gray-200">{stats.scored}</dd>
            <dt className="text-gray-400">{t('settings:junk.unscored')}</dt>
            <dd className="text-right tabular-nums text-gray-200">{stats.unscored}</dd>
            <dt className="text-gray-400">{t('settings:junk.foundSpam')}</dt>
            <dd className="text-right tabular-nums text-gray-200">{stats.spam}</dd>
            <dt className="text-gray-400">{t('settings:junk.foundGraymail')}</dt>
            <dd className="text-right tabular-nums text-gray-200">{stats.graymail}</dd>
            {config.phishingEnabled && (
              <>
                <dt className="text-gray-400">{t('settings:junk.foundPhishing')}</dt>
                <dd className="text-right tabular-nums text-gray-200">{stats.phishing}</dd>
              </>
            )}
            <dt className="text-gray-400">{t('settings:junk.yourFeedback')}</dt>
            <dd className="text-right tabular-nums text-gray-200">
              {stats.markedJunk} / {stats.markedNotJunk}
            </dd>
          </dl>

          <div className="mt-3 space-y-1 border-t border-gray-700 pt-3">
            {stats.models
              .filter((m) => isModelAxis(m.axis))
              .map((m) => (
                <div key={m.axis} className="flex items-baseline justify-between gap-4 text-xs">
                  <span className="text-gray-400">{t(`settings:junk.model.${m.axis as ModelAxis}` as const)}</span>
                  {/* Amber, not red: "not enough labels yet" is the expected
                      state on a young mailbox, not a fault. */}
                  <span className={m.inUse ? 'text-right text-gray-300' : 'text-right text-amber-400'}>
                    {m.inUse
                      ? t('settings:junk.modelInUse', { pos: m.positives, neg: m.negatives })
                      : t('settings:junk.modelNotYet', { pos: m.positives, neg: m.negatives })}
                  </span>
                </div>
              ))}
          </div>

          <button
            type="button"
            onClick={() => void handleBackfill()}
            disabled={backfilling || !config.enabled || stats.unscored === 0}
            className="mt-3 rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600 disabled:opacity-50"
          >
            {t('settings:junk.backfill', { count: stats.unscored })}
          </button>
        </section>
      )}
    </div>
  );
}
