import { useTranslation } from 'react-i18next';
import type { CatalogModel, ModelDownloadProgress } from '@/types';
import { ModelRow } from './ModelRow';
import { ThinkingToggle } from './ThinkingToggle';
import type { AiConfigState } from './types';

interface EmbeddedPanelProps {
  config: AiConfigState;
  setConfig: (next: AiConfigState) => void;
  catalog: CatalogModel[];
  downloads: Record<string, ModelDownloadProgress>;
  onSelectModel: (model: CatalogModel) => void;
  onDownload: (modelId: string) => void;
  onCancel: (modelId: string) => void;
  onDelete: (model: CatalogModel) => void;
}

/**
 * Embedded llama.cpp panel: in-process GGUF inference. Lists chat + embedding
 * catalog models with download/select/delete affordances and a thinking toggle.
 */
export function EmbeddedPanel({
  config,
  setConfig,
  catalog,
  downloads,
  onSelectModel,
  onDownload,
  onCancel,
  onDelete,
}: EmbeddedPanelProps) {
  const { t } = useTranslation(['common', 'settings']);
  const chatCatalog = catalog.filter((m) => m.kind === 'chat');
  const embedCatalog = catalog.filter((m) => m.kind === 'embedding');
  const hasLocalChat = chatCatalog.some((m) => m.isLocal);
  const hasLocalEmbed = embedCatalog.some((m) => m.isLocal);

  return (
    <div className="space-y-4">
      {catalog.length === 0 ? (
        <p className="text-sm text-gray-500">{t('settings:ai.noCatalog')}</p>
      ) : (
        <>
          {/* Chat models */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <label className="text-sm font-medium text-gray-300">{t('settings:ai.chatModel')}</label>
              {!hasLocalChat && <span className="text-xs text-gray-500">{t('settings:ai.downloadOneToStart')}</span>}
            </div>
            <div className="space-y-2">
              {chatCatalog.map((m) => (
                <ModelRow
                  key={m.id}
                  model={m}
                  isSelected={config.model === m.id}
                  downloadProgress={downloads[m.id] ?? null}
                  onSelect={() => onSelectModel(m)}
                  onDownload={() => onDownload(m.id)}
                  onCancel={() => onCancel(m.id)}
                  onDelete={() => onDelete(m)}
                />
              ))}
            </div>
          </div>

          {/* Embedding models */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <label className="text-sm font-medium text-gray-300">{t('settings:ai.embeddingModel')}</label>
              {!hasLocalEmbed && <span className="text-xs text-gray-500">{t('settings:ai.embeddingRequired')}</span>}
            </div>
            <div className="space-y-2">
              {embedCatalog.map((m) => (
                <ModelRow
                  key={m.id}
                  model={m}
                  isSelected={config.embeddingModel === m.id}
                  downloadProgress={downloads[m.id] ?? null}
                  onSelect={() => onSelectModel(m)}
                  onDownload={() => onDownload(m.id)}
                  onCancel={() => onCancel(m.id)}
                  onDelete={() => onDelete(m)}
                />
              ))}
            </div>
          </div>
        </>
      )}

      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
