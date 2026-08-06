import { listen } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import * as api from '@/lib/api';
import { ALL_SOURCES, type LogSource, useLogStore } from '@/stores/logStore';
import type { CatalogModel } from '@/types';
import { LogEntryList } from './LogEntryList';

type Provider = 'llamacpp' | 'ollama' | 'openrouter';

const PROVIDER_LABELS: Record<Provider, string> = {
  llamacpp: 'Embedded',
  ollama: 'Ollama',
  openrouter: 'OpenRouter',
};

function ModelSelector() {
  const { t } = useTranslation(['dashboard']);
  const [provider, setProvider] = useState<Provider>('ollama');
  const [models, setModels] = useState<string[]>([]);
  const [currentModel, setCurrentModel] = useState<string>('');
  const addLog = useLogStore((s) => s.addLog);

  const loadModels = async (prov: Provider): Promise<string[]> => {
    if (prov === 'ollama') {
      const all = await api.listOllamaModels().catch(() => [] as string[]);
      return all.filter((m) => !/(embed|nomic|bge|e5)/i.test(m));
    }
    if (prov === 'llamacpp') {
      const catalog = await api.listCatalogModels().catch(() => [] as CatalogModel[]);
      return catalog.filter((m) => m.kind === 'chat' && m.isLocal).map((m) => m.id);
    }
    // openrouter: no fixed list — model is a free-form string configured in AI Settings.
    return [];
  };

  const load = async () => {
    const cfg = await api.getAiConfig().catch(() => null);
    const prov = (cfg?.provider as Provider | undefined) ?? 'ollama';
    setProvider(prov);
    const list = await loadModels(prov);
    setModels(list);
    const savedModel = cfg?.model ?? '';
    // Use saved model if it's in the list, else first item
    setCurrentModel(list.includes(savedModel) ? savedModel : (list[0] ?? savedModel));
  };

  useEffect(() => {
    void load();
    // Refresh when AI Settings saves (provider/model may have changed)
    const unlistenConfig = listen('ai-config-updated', () => void load());
    // Refresh when any model download completes so newly-downloaded models
    // appear in the dropdown immediately (ai-config-updated only fires on
    // auto-selection, not on every download)
    const unlistenDownload = listen<{ status: string }>('model-download-progress', (e) => {
      if (e.payload?.status === 'complete') void load();
    });
    return () => {
      void unlistenConfig.then((u) => u());
      void unlistenDownload.then((u) => u());
    };
  }, []);

  const handleProviderChange = async (newProv: Provider) => {
    setProvider(newProv);
    const list = await loadModels(newProv);
    setModels(list);
    const newModel = list[0] ?? '';
    setCurrentModel(newModel);
    // Persist both provider and model
    try {
      await api.setPref('ai_provider', newProv);
      if (newModel) await api.setAiModel(newModel);
      addLog('info', 'ai', `AI backend → ${PROVIDER_LABELS[newProv]}${newModel ? ` · ${newModel}` : ''}`);
    } catch (err) {
      addLog('error', 'ai', `Failed to switch provider: ${err}`);
    }
  };

  const handleModelChange = async (model: string) => {
    setCurrentModel(model);
    try {
      await api.setAiModel(model);
      addLog('info', 'ai', `AI model → ${model}`);
    } catch (err) {
      addLog('error', 'ai', `Failed to set model: ${err}`);
    }
  };

  return (
    <div className="flex items-center gap-1">
      {/* Provider selector */}
      <Select
        value={provider}
        onChange={(value) => void handleProviderChange(value)}
        options={(Object.keys(PROVIDER_LABELS) as Provider[]).map((p) => ({ value: p, label: PROVIDER_LABELS[p] }))}
        ariaLabel={t('dashboard:log.aiBackend')}
        size="xs"
      />

      {/* Model selector — hidden for openrouter (free-form model configured in AI Settings). */}
      {provider !== 'openrouter' && models.length > 0 && (
        <Select
          value={currentModel}
          onChange={(value) => void handleModelChange(value)}
          options={models.map((m) => ({ value: m, label: m }))}
          ariaLabel={t('dashboard:log.aiModel')}
          size="xs"
        />
      )}
    </div>
  );
}

export function LogPanel({ onOpenAiSettings }: { onOpenAiSettings?: () => void }) {
  const { t } = useTranslation(['dashboard']);
  const { entries, isOpen, toggle, clear } = useLogStore();
  const [sourceFilter, setSourceFilter] = useState<LogSource | 'all'>('all');

  const visibleEntries = sourceFilter === 'all' ? entries : entries.filter((e) => e.source === sourceFilter);

  return (
    <div className="flex flex-col border-t border-gray-700 bg-[#1e1e1e] text-gray-300 text-xs font-mono">
      {/* Header bar */}
      <div className="flex items-center justify-between px-3 py-1.5 bg-[#252526] border-b border-gray-700 select-none">
        <div className="flex items-center gap-3">
          <button onClick={toggle} className="flex items-center gap-1.5 hover:text-white transition-colors">
            <svg
              className={`w-3 h-3 transition-transform ${isOpen ? 'rotate-0' : '-rotate-90'}`}
              fill="currentColor"
              viewBox="0 0 20 20"
            >
              <path
                fillRule="evenodd"
                d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z"
                clipRule="evenodd"
              />
            </svg>
            <span className="text-[11px] font-semibold tracking-wide uppercase">{t('dashboard:log.title')}</span>
          </button>
          {entries.length > 0 && (
            <span className="text-[10px] text-gray-500">
              {visibleEntries.length}
              {sourceFilter !== 'all' && ` / ${entries.length}`} {visibleEntries.length === 1 ? 'entry' : 'entries'}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2">
          {/* Source filter */}
          <Select
            value={sourceFilter}
            onChange={setSourceFilter}
            options={[
              { value: 'all' as const, label: t('dashboard:log.allModules') },
              ...ALL_SOURCES.map((s) => ({ value: s, label: s })),
            ]}
            ariaLabel={t('dashboard:log.filterByModule')}
            size="xs"
          />
          <ModelSelector />
          <button
            onClick={onOpenAiSettings}
            className="p-1 hover:bg-gray-600/50 rounded transition-colors"
            title={t('dashboard:log.aiSettings')}
          >
            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
              />
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            </svg>
          </button>
          {/* Clear button */}
          <button
            onClick={clear}
            className="p-1 hover:bg-gray-600/50 rounded transition-colors"
            title={t('dashboard:log.clearLogs')}
          >
            <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
              />
            </svg>
          </button>

          {/* Toggle button */}
          <button
            onClick={toggle}
            className="p-1 hover:bg-gray-600/50 rounded transition-colors"
            title={isOpen ? t('dashboard:log.minimizePanel') : t('dashboard:log.expandPanel')}
          >
            {isOpen ? (
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            ) : (
              <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 15l7-7 7 7" />
              </svg>
            )}
          </button>
        </div>
      </div>

      {/* Log content */}
      {isOpen && (
        <LogEntryList
          entries={visibleEntries}
          emptyLabel={
            entries.length === 0
              ? t('dashboard:log.empty')
              : t('dashboard:log.emptyForModule', { module: sourceFilter })
          }
          className="h-44"
        />
      )}
    </div>
  );
}
