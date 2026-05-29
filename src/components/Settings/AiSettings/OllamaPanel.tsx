import { useTranslation } from 'react-i18next';
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
  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.chatModel')}</label>
        <select
          value={config.model}
          onChange={(e) => setConfig({ ...config, model: e.target.value })}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        >
          {ollamaModels.length === 0 && (
            <option value={config.model}>{config.model || t('settings:ai.noModelsFound')}</option>
          )}
          {ollamaModels.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.embeddingModel')}</label>
        <select
          value={config.embeddingModel}
          onChange={(e) => setConfig({ ...config, embeddingModel: e.target.value })}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        >
          {ollamaEmbedModels.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </div>
      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
