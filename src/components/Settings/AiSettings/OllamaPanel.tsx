import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import { ThinkingToggle } from './ThinkingToggle';
import type { AiConfigState } from './types';

interface OllamaPanelProps {
  config: AiConfigState;
  setConfig: (next: AiConfigState) => void;
  ollamaModels: string[];
  ollamaEmbedModels: string[];
}

/**
 * Local Ollama server panel — picks chat + embedding models from the running
 * Ollama daemon (filtered into chat/embedding lists by name heuristics in the
 * parent).
 */
export function OllamaPanel({ config, setConfig, ollamaModels, ollamaEmbedModels }: OllamaPanelProps) {
  const { t } = useTranslation(['common', 'settings']);
  const chatModelOptions =
    ollamaModels.length === 0
      ? [{ value: config.model, label: config.model || t('settings:ai.noModelsFound') }]
      : ollamaModels.map((m) => ({ value: m, label: m }));
  const embedModelOptions = ollamaEmbedModels.map((m) => ({ value: m, label: m }));
  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.chatModel')}</label>
        <Select
          value={config.model}
          options={chatModelOptions}
          onChange={(value) => setConfig({ ...config, model: value })}
          ariaLabel={t('settings:ai.chatModel')}
          fullWidth
        />
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.embeddingModel')}</label>
        <Select
          value={config.embeddingModel}
          options={embedModelOptions}
          onChange={(value) => setConfig({ ...config, embeddingModel: value })}
          ariaLabel={t('settings:ai.embeddingModel')}
          fullWidth
        />
      </div>
      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
