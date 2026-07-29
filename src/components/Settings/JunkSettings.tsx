import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { JunkConfig, JunkFlaggedAction, JunkStats } from '@/types';

/** Axes with a trained model. Typed so a missing translation fails the build. */
const MODEL_AXES = ['spam', 'graymail'] as const;
type ModelAxis = (typeof MODEL_AXES)[number];

function isModelAxis(axis: string): axis is ModelAxis {
  return (MODEL_AXES as readonly string[]).includes(axis);
}

import { ToggleSwitch } from '@/components/common/ToggleSwitch';

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
  const [config, setConfig] = useState<JunkConfig>(DEFAULT_CONFIG);
  const [stats, setStats] = useState<JunkStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [backfilling, setBackfilling] = useState(false);

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
    <div className="space-y-6">
      {/* Errors sit above everything, never at the bottom of a scroll container. */}
      {error && <div className="rounded border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-800">{error}</div>}

      <section className="space-y-3">
        <ToggleSwitch
          checked={config.enabled}
          onChange={(v) => void save({ ...config, enabled: v })}
          label={t('settings:junk.enabled')}
          description={t('settings:junk.enabledDesc')}
        />
      </section>

      <section className="space-y-3">
        <h3 className="text-sm font-medium text-gray-900">{t('settings:junk.flaggedActionTitle')}</h3>
        <div className="space-y-2">
          {ACTIONS.map((action) => (
            <label key={action} className="flex items-start gap-2 cursor-pointer">
              <input
                type="radio"
                name="junk-flagged-action"
                className="mt-1"
                checked={config.flaggedAction === action}
                onChange={() => void save({ ...config, flaggedAction: action })}
                disabled={!config.enabled}
              />
              <span className="text-sm">
                <span className="text-gray-900">{t(`settings:junk.action.${action}` as const)}</span>
                <span className="block text-xs text-gray-500">{t(`settings:junk.actionDesc.${action}` as const)}</span>
              </span>
            </label>
          ))}
        </div>
        <p className="text-xs text-gray-500">{t('settings:junk.neverMoves')}</p>
      </section>

      <section className="space-y-3">
        <ToggleSwitch
          checked={config.phishingEnabled}
          onChange={(v) => void save({ ...config, phishingEnabled: v })}
          label={t('settings:junk.phishing')}
          description={t('settings:junk.phishingDesc')}
        />
      </section>

      {stats && (
        <section className="space-y-2 rounded border border-gray-200 bg-gray-50 px-3 py-3">
          <h3 className="text-sm font-medium text-gray-900">{t('settings:junk.statusTitle')}</h3>
          <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-xs text-gray-700">
            <dt>{t('settings:junk.scored')}</dt>
            <dd className="text-right tabular-nums">{stats.scored}</dd>
            <dt>{t('settings:junk.unscored')}</dt>
            <dd className="text-right tabular-nums">{stats.unscored}</dd>
            <dt>{t('settings:junk.foundSpam')}</dt>
            <dd className="text-right tabular-nums">{stats.spam}</dd>
            <dt>{t('settings:junk.foundGraymail')}</dt>
            <dd className="text-right tabular-nums">{stats.graymail}</dd>
            {config.phishingEnabled && (
              <>
                <dt>{t('settings:junk.foundPhishing')}</dt>
                <dd className="text-right tabular-nums">{stats.phishing}</dd>
              </>
            )}
            <dt>{t('settings:junk.yourFeedback')}</dt>
            <dd className="text-right tabular-nums">
              {stats.markedJunk} / {stats.markedNotJunk}
            </dd>
          </dl>

          <div className="pt-1 space-y-1">
            {stats.models
              .filter((m) => isModelAxis(m.axis))
              .map((m) => (
                <div key={m.axis} className="flex items-baseline justify-between text-xs">
                  <span className="text-gray-700">{t(`settings:junk.model.${m.axis as ModelAxis}` as const)}</span>
                  <span className={m.inUse ? 'text-gray-600' : 'text-amber-700'}>
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
            className="mt-2 rounded border border-gray-300 bg-white px-3 py-1 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50"
          >
            {t('settings:junk.backfill', { count: stats.unscored })}
          </button>
        </section>
      )}
    </div>
  );
}
