import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import * as api from '@/lib/api';
import { type LogLevel, type LogSource, useLogStore } from '@/stores/logStore';
import type { CatalogModel } from '@/types';

/** Options matching the log panel's fixed 24-hour HH:MM:SS time format. */
const LOG_TIME_OPTIONS: Intl.DateTimeFormatOptions = {
  hour12: false,
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
};

const ALL_SOURCES: LogSource[] = [
  'sync',
  'ai',
  'search',
  'account',
  'system',
  'embeddings',
  'attachments',
  'chat',
  'memory',
  'tasks',
  'lens',
];

function LevelIcon({ level }: { level: LogLevel }) {
  const { t } = useTranslation(['dashboard']);
  switch (level) {
    case 'error':
      return <span className="text-red-400 text-xs font-bold">{t('dashboard:log.levels.error')}</span>;
    case 'warn':
      return <span className="text-yellow-400 text-xs font-bold">{t('dashboard:log.levels.warn')}</span>;
    case 'success':
      return <span className="text-green-400 text-xs font-bold">{t('dashboard:log.levels.success')}</span>;
    case 'debug':
      return <span className="text-gray-500 text-xs font-bold">{t('dashboard:log.levels.debug')}</span>;
    default:
      return <span className="text-blue-400 text-xs font-bold">{t('dashboard:log.levels.info')}</span>;
  }
}

function SourceBadge({ source }: { source: LogSource }) {
  const colors: Record<LogSource, string> = {
    sync: 'bg-indigo-900/50 text-indigo-300',
    ai: 'bg-purple-900/50 text-purple-300',
    search: 'bg-cyan-900/50 text-cyan-300',
    account: 'bg-amber-900/50 text-amber-300',
    system: 'bg-gray-700/50 text-gray-300',
    embeddings: 'bg-emerald-900/50 text-emerald-300',
    attachments: 'bg-orange-900/50 text-orange-300',
    chat: 'bg-sky-900/50 text-sky-300',
    memory: 'bg-pink-900/50 text-pink-300',
    tasks: 'bg-rose-900/50 text-rose-300',
    lens: 'bg-violet-900/50 text-violet-300',
  };

  return (
    <span className={`px-1.5 py-0.5 rounded text-[10px] font-medium uppercase tracking-wide ${colors[source]}`}>
      {source}
    </span>
  );
}

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

  const selectCls =
    'bg-[#333] text-gray-300 text-[10px] border border-gray-600 rounded px-1.5 py-0.5 outline-none hover:border-gray-500 focus:border-primary-500 cursor-pointer';

  return (
    <div className="flex items-center gap-1">
      {/* Provider selector */}
      <select
        value={provider}
        onChange={(e) => void handleProviderChange(e.target.value as Provider)}
        className={selectCls}
        title={t('dashboard:log.aiBackend')}
      >
        {(Object.keys(PROVIDER_LABELS) as Provider[]).map((p) => (
          <option key={p} value={p}>
            {PROVIDER_LABELS[p]}
          </option>
        ))}
      </select>

      {/* Model selector — hidden for openrouter (free-form model configured in AI Settings). */}
      {provider !== 'openrouter' && models.length > 0 && (
        <select
          value={currentModel}
          onChange={(e) => void handleModelChange(e.target.value)}
          className={selectCls}
          title={t('dashboard:log.aiModel')}
        >
          {models.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      )}
    </div>
  );
}

export function LogPanel({ onOpenAiSettings }: { onOpenAiSettings?: () => void }) {
  const { t } = useTranslation(['dashboard']);
  const fmt = useFormatters();
  const { entries, isOpen, toggle, clear } = useLogStore();
  const [sourceFilter, setSourceFilter] = useState<LogSource | 'all'>('all');
  const scrollRef = useRef<HTMLDivElement>(null);
  const wasAtBottomRef = useRef(true);

  const visibleEntries = sourceFilter === 'all' ? entries : entries.filter((e) => e.source === sourceFilter);

  // Track if user is at bottom before new entries arrive
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const handleScroll = () => {
      wasAtBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    };
    el.addEventListener('scroll', handleScroll);
    return () => el.removeEventListener('scroll', handleScroll);
  }, [isOpen]);

  // Auto-scroll to bottom when new entries arrive (only if already at bottom)
  useEffect(() => {
    if (wasAtBottomRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [visibleEntries.length]);

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
          <select
            value={sourceFilter}
            onChange={(e) => setSourceFilter(e.target.value as LogSource | 'all')}
            className="bg-[#333] text-gray-300 text-[10px] border border-gray-600 rounded px-1.5 py-0.5 outline-none hover:border-gray-500 focus:border-primary-500 cursor-pointer"
            title={t('dashboard:log.filterByModule')}
          >
            <option value="all">{t('dashboard:log.allModules')}</option>
            {ALL_SOURCES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
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
        <div ref={scrollRef} className="overflow-y-auto h-44 px-1 py-1">
          {visibleEntries.length === 0 ? (
            <div className="flex items-center justify-center h-full text-gray-600">
              {entries.length === 0 ? 'No log entries yet' : `No entries for module "${sourceFilter}"`}
            </div>
          ) : (
            visibleEntries.map((entry) => (
              <div key={entry.id} className="flex items-start gap-2 px-2 py-0.5 hover:bg-white/[0.03] leading-5">
                <span className="text-gray-400 shrink-0">{fmt.time(entry.timestamp / 1000, LOG_TIME_OPTIONS)}</span>
                <span className="shrink-0 w-7 text-center">
                  <LevelIcon level={entry.level} />
                </span>
                <span className="shrink-0">
                  <SourceBadge source={entry.source} />
                </span>
                <span
                  className={
                    entry.level === 'error'
                      ? 'text-red-300'
                      : entry.level === 'warn'
                        ? 'text-yellow-200'
                        : 'text-gray-300'
                  }
                >
                  {entry.message}
                </span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
