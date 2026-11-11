import { useTranslation } from 'react-i18next';
import type { AiConfigState } from './types';
import { UsageSummary } from './UsageSummary';

interface OpenRouterSpendControlsProps {
  config: AiConfigState;
  setConfig: (next: AiConfigState) => void;
}

/**
 * The monthly budget, what was spent against it, and zero data retention:
 * shown wherever OpenRouter receives mail — the OpenRouter tab, and an
 * OpenAI-compatible server whose email index OpenRouter computes.
 */
export function OpenRouterSpendControls({ config, setConfig }: OpenRouterSpendControlsProps) {
  const { t } = useTranslation(['common', 'settings']);
  return (
    <>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:ai.monthlyBudget')}</label>
        <input
          type="number"
          min={0}
          step={0.5}
          value={config.monthlyBudgetUsd}
          onChange={(e) => setConfig({ ...config, monthlyBudgetUsd: parseFloat(e.target.value) || 0 })}
          className="w-full bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        />
        <p className="text-xs text-gray-500 mt-1">{t('settings:ai.monthlyBudgetHelp')}</p>
      </div>
      {/* Directly under the cap it reports against, so hitting the budget and
          finding out what you spent are the same screen. */}
      <UsageSummary />
      <p className="text-xs text-gray-500">{t('settings:openRouter.noTrainingNotice')}</p>
      <div className="flex items-center justify-between">
        <div>
          <label className="block text-sm font-medium text-gray-700">
            {t('settings:openRouter.zeroDataRetention')}
          </label>
          <p className="text-xs text-gray-500 mt-0.5">{t('settings:openRouter.zeroDataRetentionHelp')}</p>
        </div>
        <button
          type="button"
          aria-label={t('settings:openRouter.zeroDataRetention')}
          aria-pressed={config.zeroDataRetention}
          onClick={() => setConfig({ ...config, zeroDataRetention: !config.zeroDataRetention })}
          className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
            config.zeroDataRetention ? 'bg-primary-600' : 'bg-gray-300'
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
              config.zeroDataRetention ? 'translate-x-5' : 'translate-x-0'
            }`}
          />
        </button>
      </div>
    </>
  );
}
