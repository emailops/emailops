// Spend in the current AI accounting period, with a control to start a new one.
//
// The backend has enforced a monthly budget since it shipped -- `check_budget`
// raises `BudgetExceeded` and every AI call stops -- but nothing ever read
// `get_ai_usage` or called `reset_ai_usage`. A user who set a budget and hit it
// saw AI stop working with no way to see what they had spent and no way to
// start a new period short of editing SQLite.

import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { AiUsageSummary } from '@/types';

export function UsageSummary() {
  const { t, i18n } = useTranslation(['common', 'settings']);
  const addLog = useLogStore((s) => s.addLog);
  const [usage, setUsage] = useState<AiUsageSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isResetting, setIsResetting] = useState(false);

  const load = useCallback(async () => {
    try {
      setUsage(await api.getAiUsage());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const handleReset = useCallback(async () => {
    setIsResetting(true);
    try {
      await api.resetAiUsage();
      await load();
      addLog('success', 'ai', 'AI spending period reset.');
    } catch (e) {
      const message = errorText(e);
      setError(message);
      addLog('error', 'ai', `Failed to reset the AI spending period: ${message}`);
    } finally {
      setIsResetting(false);
    }
  }, [load, addLog]);

  if (error) {
    return (
      <div role="alert" className="rounded border border-red-800 bg-red-950/40 px-3 py-2 text-xs text-red-300">
        {error}
      </div>
    );
  }

  if (!usage) {
    return <p className="text-xs text-gray-500">{t('settings:ai.usage.loading')}</p>;
  }

  const money = (value: number) =>
    new Intl.NumberFormat(i18n.language, { style: 'currency', currency: 'USD' }).format(value);
  const count = (value: number) => new Intl.NumberFormat(i18n.language).format(value);
  // `periodStart` is 0 on an install that has never recorded a call.
  const since = usage.periodStart > 0 ? new Date(usage.periodStart * 1000).toLocaleDateString(i18n.language) : null;
  const overBudget = usage.budgetUsd > 0 && usage.totalCostUsd >= usage.budgetUsd;

  return (
    <div className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3 space-y-2">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-xs font-semibold text-gray-400 uppercase tracking-wide">
          {t('settings:ai.usage.title')}
        </span>
        <button
          type="button"
          onClick={handleReset}
          disabled={isResetting}
          className="text-xs text-primary-400 hover:text-primary-300 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {isResetting ? t('settings:ai.usage.resetting') : t('settings:ai.usage.reset')}
        </button>
      </div>

      <div className={`text-sm ${overBudget ? 'text-red-400' : 'text-gray-200'}`}>
        {usage.budgetUsd > 0
          ? t('settings:ai.usage.spentOfBudget', { spent: money(usage.totalCostUsd), budget: money(usage.budgetUsd) })
          : t('settings:ai.usage.spent', { spent: money(usage.totalCostUsd) })}
      </div>

      <p className="text-xs text-gray-500">
        {t('settings:ai.usage.detail', {
          calls: count(usage.totalCalls),
          prompt: count(usage.totalPromptTokens),
          completion: count(usage.totalCompletionTokens),
        })}
        {since && <> · {t('settings:ai.usage.since', { date: since })}</>}
      </p>

      {overBudget && <p className="text-xs text-red-400">{t('settings:ai.usage.exceeded')}</p>}
    </div>
  );
}
