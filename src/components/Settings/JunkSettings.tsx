import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ToggleSwitch } from '@/components/common/ToggleSwitch';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAccountStore } from '@/stores/accountStore';
import { useJunkStore } from '@/stores/junkStore';
import { useLogStore } from '@/stores/logStore';
import type { JunkConfig, JunkFlaggedAction, JunkStats } from '@/types';

/** Axes with a trained model. Typed so a missing translation fails the build. */
const MODEL_AXES = ['spam', 'graymail'] as const;
type ModelAxis = (typeof MODEL_AXES)[number];

function isModelAxis(axis: string): axis is ModelAxis {
  return (MODEL_AXES as readonly string[]).includes(axis);
}

// Mirrors the backend default (`services/junk/config.rs`). Off: the detector
// accuses mail of being junk and fades or hides rows on the strength of that,
// which is not something to start doing to an install that never asked.
const DEFAULT_CONFIG: JunkConfig = {
  enabled: false,
  phishingEnabled: false,
  flaggedAction: 'dim',
};

const ACTIONS: JunkFlaggedAction[] = ['dim', 'hide'];

export function JunkSettings() {
  const { t } = useTranslation(['settings', 'common']);
  const addLog = useLogStore((s) => s.addLog);
  const accounts = useAccountStore((s) => s.accounts);
  const [config, setConfig] = useState<JunkConfig>(DEFAULT_CONFIG);
  // Keyed by account id, because every number in the status block is
  // account-scoped: the counts, the trained models and the backfill queue. The
  // panel used to report on one account only — whichever the rest of the app
  // happened to have selected — so on a multi-mailbox install it silently
  // described a mailbox the reader had no way to identify.
  const [statsByAccount, setStatsByAccount] = useState<Record<string, JunkStats>>({});
  const [error, setError] = useState<string | null>(null);
  const [backfilling, setBackfilling] = useState<Record<string, true>>({});

  // Disabled accounts sync nothing, so there is nothing to score in them and no
  // status worth reporting.
  const scoredAccounts = accounts.filter((a) => a.enabled);
  // Effects below depend on *which* accounts, not on the array identity the
  // store hands back on every unrelated update.
  const accountKey = scoredAccounts.map((a) => a.id).join(',');

  useEffect(() => {
    api
      .getJunkConfig()
      .then((loaded) => {
        setConfig(loaded);
        useJunkStore.setState({ flaggedAction: loaded.flaggedAction });
      })
      .catch((e) => setError(errorText(e)));
  }, []);

  const refreshStats = useCallback(async (accountIds: string[]) => {
    const entries = await Promise.all(
      accountIds.map(async (id) => {
        try {
          return [id, await api.getJunkStats(id)] as const;
        } catch {
          // One unreadable account must not blank out the others.
          return null;
        }
      }),
    );
    setStatsByAccount(Object.fromEntries(entries.filter((e): e is [string, JunkStats] => e !== null)));
  }, []);

  useEffect(() => {
    void refreshStats(accountKey ? accountKey.split(',') : []);
  }, [refreshStats, accountKey]);

  const save = useCallback(
    async (next: JunkConfig) => {
      const previous = config;
      setConfig(next);
      try {
        await api.setJunkConfig(next);
        // The inbox decides whether to drop flagged rows from `useJunkStore`,
        // and the store reads this preference exactly once — at app start.
        // Persisting it is not enough: without publishing it here, ticking
        // "keep it out of the inbox" changes nothing on screen until the next
        // launch, and the checkbox above the list keeps showing unticked.
        useJunkStore.setState({ flaggedAction: next.flaggedAction });
      } catch (e) {
        // Roll back rather than leave the switch showing a state the backend
        // never accepted.
        setConfig(previous);
        setError(errorText(e));
      }
    },
    [config],
  );

  const handleBackfill = useCallback(
    async (accountId: string) => {
      setBackfilling((b) => ({ ...b, [accountId]: true }));
      try {
        await api.backfillJunkScores(accountId);
        addLog('info', 'system', t('settings:junk.backfillStarted'));
      } catch (e) {
        setError(errorText(e));
        setBackfilling((b) => {
          const { [accountId]: _done, ...rest } = b;
          return rest;
        });
      }
    },
    [addLog, t],
  );

  // The command returns as soon as the work is queued, so without this the
  // panel keeps showing the same "score N older messages" and the click looks
  // like it did nothing. Poll until the queue drains, then stop — a spinner
  // that never resolves is its own kind of broken. Accounts can be backfilling
  // at once, so this watches whichever set is currently running.
  const runningKey = Object.keys(backfilling).sort().join(',');
  useEffect(() => {
    if (!runningKey) return;
    const running = runningKey.split(',');
    let cancelled = false;
    let ticks = 0;
    const timer = window.setInterval(async () => {
      ticks += 1;
      for (const accountId of running) {
        try {
          const next = await api.getJunkStats(accountId);
          if (cancelled) return;
          setStatsByAccount((s) => ({ ...s, [accountId]: next }));
          // Give up watching after ~2 minutes. The work continues in the
          // background either way; only the live counter stops.
          if (next.unscored === 0 || ticks > 60) {
            setBackfilling((b) => {
              const { [accountId]: _done, ...rest } = b;
              return rest;
            });
          }
        } catch {
          if (cancelled) return;
          setBackfilling((b) => {
            const { [accountId]: _done, ...rest } = b;
            return rest;
          });
        }
      }
    }, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [runningKey]);

  // Same two-level shape as every sibling panel: the tab body in
  // SettingsDialog has no padding or scrolling of its own, so each panel owns
  // its `overflow-y-auto flex-1 px-6 py-5` container.
  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
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
                  <span className="block text-xs text-gray-500">
                    {t(`settings:junk.actionDesc.${action}` as const)}
                  </span>
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

        {/* One card per account. Every figure below is per-mailbox, so a single
            merged block would be a number nobody could act on. "Status" is the
            heading for the group, not for each card — repeating it once per
            mailbox buries the thing the reader is scanning for, which is the
            address. */}
        <div className="space-y-3">
          {scoredAccounts.some((a) => statsByAccount[a.id]) && (
            <h3 className="text-sm font-semibold text-gray-300">{t('settings:junk.statusTitle')}</h3>
          )}
          {scoredAccounts.map((account) => {
            const stats = statsByAccount[account.id];
            if (!stats) return null;
            const isBackfilling = backfilling[account.id] === true;
            return (
              <section key={account.id} className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
                <h4 className="mb-2 text-sm font-medium text-gray-200">{account.email}</h4>

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
                        <span className="text-gray-400">
                          {t(`settings:junk.model.${m.axis as ModelAxis}` as const)}
                        </span>
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
                  onClick={() => void handleBackfill(account.id)}
                  disabled={isBackfilling || !config.enabled || stats.unscored === 0}
                  className="mt-3 rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600 disabled:opacity-50"
                >
                  {isBackfilling
                    ? t('settings:junk.backfillRunning', { count: stats.unscored })
                    : t('settings:junk.backfill', { count: stats.unscored })}
                </button>
              </section>
            );
          })}
        </div>
      </div>
    </div>
  );
}
