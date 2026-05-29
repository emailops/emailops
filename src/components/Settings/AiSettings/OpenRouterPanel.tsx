import { useTranslation } from 'react-i18next';
import { ThinkingToggle } from './ThinkingToggle';
import type { AiConfigState } from './types';

interface OpenRouterPanelProps {
  config: AiConfigState;
  setConfig: (next: AiConfigState) => void;
  apiKey: string;
  setApiKey: (key: string) => void;
}

/**
 * Cloud OpenRouter panel — API key, free-form chat model id, and an optional
 * monthly USD budget cap. No embedding model field: OpenRouter is chat-only,
 * embeddings always run locally via the configured embedded backend.
 */
export function OpenRouterPanel({ config, setConfig, apiKey, setApiKey }: OpenRouterPanelProps) {
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
      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
