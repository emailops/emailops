import { useTranslation } from 'react-i18next';
import { MIN_CONTEXT_BUDGET } from './helpers';
import { ThinkingToggle } from './ThinkingToggle';
import type { AiConfigState } from './types';
import { UsageSummary } from './UsageSummary';

interface OpenRouterPanelProps {
  config: AiConfigState;
  setConfig: (next: AiConfigState) => void;
  apiKey: string;
  setApiKey: (key: string) => void;
  /** Prompt budget (tokens) for remote models, `chat.remote_n_ctx_budget`. */
  contextBudget: number;
  onContextBudgetChange: (tokens: number) => void;
}

/**
 * Cloud OpenRouter panel — API key, free-form chat model id, and an optional
 * monthly USD budget cap. No embedding model field: OpenRouter is chat-only,
 * embeddings always run locally via the configured embedded backend.
 *
 * Requests always forbid providers that train on or store prompts (fixed in
 * the backend, see `ai/openrouter.rs`); zero data retention is the user's call
 * because it rules out many models.
 */
export function OpenRouterPanel({
  config,
  setConfig,
  apiKey,
  setApiKey,
  contextBudget,
  onContextBudgetChange,
}: OpenRouterPanelProps) {
  const { t } = useTranslation(['common', 'settings']);
  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">
          {t('settings:ai.apiKey')}
          {config.hasApiKey && <span className="text-gray-500 font-normal"> {t('settings:ai.apiKeySaved')}</span>}
        </label>
        <input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={config.hasApiKey ? '••••••••••••••••' : t('settings:openRouter.apiKeyPlaceholder')}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
        />
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.chatModel')}</label>
        <input
          type="text"
          value={config.model}
          onChange={(e) => setConfig({ ...config, model: e.target.value })}
          placeholder={t('settings:openRouter.chatModelPlaceholder')}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
        />
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.monthlyBudget')}</label>
        <input
          type="number"
          min={0}
          step={0.5}
          value={config.monthlyBudgetUsd}
          onChange={(e) => setConfig({ ...config, monthlyBudgetUsd: parseFloat(e.target.value) || 0 })}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        />
        <p className="text-xs text-gray-500 mt-1">{t('settings:ai.monthlyBudgetHelp')}</p>
      </div>
      {/* Directly under the cap it reports against, so hitting the budget and
          finding out what you spent are the same screen. */}
      <UsageSummary />
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:openRouter.contextBudget')}</label>
        <p className="text-xs text-gray-500 mb-2">{t('settings:openRouter.contextBudgetHelp')}</p>
        <input
          type="number"
          aria-label={t('settings:openRouter.contextBudget')}
          min={MIN_CONTEXT_BUDGET}
          step={4096}
          value={contextBudget}
          onChange={(e) => {
            const v = parseInt(e.target.value, 10);
            if (Number.isFinite(v)) onContextBudgetChange(v);
          }}
          className="w-32 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        />
      </div>
      <p className="text-xs text-gray-500">{t('settings:openRouter.noTrainingNotice')}</p>
      <div className="flex items-center justify-between">
        <div>
          <label className="block text-sm font-medium text-gray-300">
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
            config.zeroDataRetention ? 'bg-primary-600' : 'bg-gray-600'
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
              config.zeroDataRetention ? 'translate-x-5' : 'translate-x-0'
            }`}
          />
        </button>
      </div>
      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
