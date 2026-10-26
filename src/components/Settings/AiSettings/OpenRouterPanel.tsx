import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import type { AiModelInfo } from '@/types';
import { MIN_CONTEXT_BUDGET } from './helpers';
import { OpenRouterSpendControls } from './OpenRouterSpendControls';
import { openRouterEmbeddingSelectOptions } from './openRouterEmbeddingOptions';
import { ThinkingToggle } from './ThinkingToggle';
import type { AiConfigState } from './types';

interface OpenRouterPanelProps {
  config: AiConfigState;
  setConfig: (next: AiConfigState) => void;
  apiKey: string;
  setApiKey: (key: string) => void;
  /** Prompt budget (tokens) for remote models, `chat.remote_n_ctx_budget`. */
  contextBudget: number;
  onContextBudgetChange: (tokens: number) => void;
  /** OpenRouter's embedding models; empty until a saved key lets them load. */
  embeddingModels: AiModelInfo[];
  /** The selected embedding model will be checked by the backend on Save. */
  embeddingNeedsCheck: boolean;
}

/**
 * Cloud OpenRouter panel — API key, free-form chat model id, an optional
 * embedding model and a monthly USD budget cap.
 *
 * With OpenRouter selected, embeddings run on OpenRouter too: the text of
 * every indexed email and every search or chat query is sent there, which is
 * why the selector starts at "none" (keyword-only search) and carries a
 * notice. A chosen model is checked on Save against the email index's vector
 * size before it is stored — see `validate_openrouter_embedding_model`.
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
  embeddingModels,
  embeddingNeedsCheck,
}: OpenRouterPanelProps) {
  const { t } = useTranslation(['common', 'settings']);
  const embeddingOptions = openRouterEmbeddingSelectOptions(t, embeddingModels, config.embeddingModel);
  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">
          {t('settings:ai.apiKey')}
          {config.hasApiKey && <span className="text-gray-500 font-normal"> {t('settings:ai.apiKeySaved')}</span>}
        </label>
        <input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={config.hasApiKey ? '••••••••••••••••' : t('settings:openRouter.apiKeyPlaceholder')}
          className="w-full bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
        />
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:ai.chatModel')}</label>
        <input
          type="text"
          value={config.model}
          onChange={(e) => setConfig({ ...config, model: e.target.value })}
          placeholder={t('settings:openRouter.chatModelPlaceholder')}
          className="w-full bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
        />
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:ai.embeddingModel')}</label>
        <Select
          value={config.embeddingModel}
          options={embeddingOptions}
          onChange={(value) => setConfig({ ...config, embeddingModel: value })}
          ariaLabel={t('settings:ai.embeddingModel')}
          fullWidth
        />
        <p className="text-xs text-gray-500 mt-1">{t('settings:openRouter.embeddingNotice')}</p>
        {!config.hasApiKey && (
          <p className="text-xs text-gray-500 mt-1">{t('settings:openRouter.embeddingNeedsKey')}</p>
        )}
        {embeddingNeedsCheck && (
          <p className="text-xs text-amber-600 mt-1">{t('settings:openRouter.embeddingNeedsCheck')}</p>
        )}
      </div>
      <OpenRouterSpendControls config={config} setConfig={setConfig} />
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:openRouter.contextBudget')}</label>
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
          className="w-32 bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        />
      </div>
      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
