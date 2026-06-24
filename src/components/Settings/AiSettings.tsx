import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useAiStore } from '@/stores/aiStore';
import { useLogStore } from '@/stores/logStore';
import type { CatalogModel, ModelDownloadProgress } from '@/types';
import { AiSharedPreferences } from './AiSettings/AiSharedPreferences';
import { ChatPromptsSection } from './AiSettings/ChatPromptsSection';
import { ConfirmDisableDialog } from './AiSettings/ConfirmDisableDialog';
import { EmbeddedPanel } from './AiSettings/EmbeddedPanel';
import { OllamaPanel } from './AiSettings/OllamaPanel';
import { OpenRouterPanel } from './AiSettings/OpenRouterPanel';
import { ProviderTab } from './AiSettings/ProviderTab';
import { type AiConfigState, DEFAULT_ROUTING_MODE, isRoutingMode, type RoutingMode } from './AiSettings/types';

interface AiSettingsProps {
  onClose: () => void;
  /** When true, render without the overlay + header chrome so it can be hosted inside a tabbed Settings dialog. */
  embedded?: boolean;
}

/**
 * Header close affordance. Module-scoped for the same reason as {@link Shell} —
 * a render-body component would remount on every parent render.
 */
function CloseButton({ onClose }: { onClose: () => void }) {
  return (
    <button onClick={onClose} className="text-gray-400 hover:text-gray-200 p-1">
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
      </svg>
    </button>
  );
}

/**
 * Wrap the settings body so it can render either as its own modal (legacy
 * callers) or embedded inside the tabbed SettingsDialog.
 *
 * Embedded mode uses `flex-1 min-h-0` (NOT `h-full`) so the flex algorithm
 * allocates the remaining height inside SettingsDialog's panel column — sibling
 * header + this panel share the column. With `h-full` the panel overflows the
 * parent and the footer's Save/Test buttons get clipped beneath the dialog edge.
 *
 * Defined at module scope, NOT inside `AiSettings`: a component declared in the
 * render body gets a fresh identity every render, so React would unmount and
 * remount this whole subtree (and reset the body's scroll position) on every
 * state change.
 */
function Shell({ embedded, children }: { embedded: boolean; children: React.ReactNode }) {
  return embedded ? (
    <div className="flex flex-col flex-1 min-h-0 w-full">{children}</div>
  ) : (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-[#252526] border border-gray-700 rounded-lg w-full max-w-2xl max-h-[90vh] flex flex-col">
        {children}
      </div>
    </div>
  );
}

/**
 * AI configuration screen. Owns provider selection, model + key state,
 * download orchestration, and assorted preferences (routing mode, keep-alive,
 * output language, chat prompts). Provider-specific UI lives in panel
 * sub-components under ./AiSettings/.
 */
export function AiSettings({ onClose, embedded = false }: AiSettingsProps) {
  const { t } = useTranslation(['common', 'settings']);
  // Master AI enable/disable — drives whether any AI command runs and whether
  // AI surfaces show up in the UI. Stored in `user_preferences.ai_enabled`.
  const { enabled: aiEnabled, setEnabled: setAiEnabled } = useAiStore();
  const [confirmDisable, setConfirmDisable] = useState(false);
  const [config, setConfig] = useState<AiConfigState | null>(null);
  const [catalog, setCatalog] = useState<CatalogModel[]>([]);
  // Map modelId → in-progress download info
  const [downloads, setDownloads] = useState<Record<string, ModelDownloadProgress>>({});
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaEmbedModels, setOllamaEmbedModels] = useState<string[]>([]);
  const [apiKey, setApiKey] = useState('');
  const [routingMode, setRoutingMode] = useState<RoutingMode>(DEFAULT_ROUTING_MODE);
  const [aiOutputLanguage, setAiOutputLanguage] = useState<string>('Spanish');
  // Minutes the local model is kept in RAM between chat turns. 0 = evict
  // immediately after use, -1 / empty-input = pin forever. Stored as seconds
  // in the `chat.keep_alive_seconds` preference.
  const [keepAliveMinutes, setKeepAliveMinutes] = useState<number>(30);
  // Cap on how far back AI processing (embeddings + classification) reaches.
  // Stored as days in the `ai_max_email_age_days` preference. 0 = no limit.
  const [aiMaxEmailAgeDays, setAiMaxEmailAgeDays] = useState<number>(365);
  // Context window (tokens) for the embedded llama.cpp chat model. Stored in
  // `chat.n_ctx`; an unset pref (or stored 0 = auto) shows the default 8192.
  // The backend clamps the saved value to [1024, model-trained-context].
  const [nCtx, setNCtx] = useState<number>(8192);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const addLog = useLogStore((s) => s.addLog);
  // Ref for catalog refresh after download complete
  const catalogRefreshRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Tracks the embedding model that was active when we last saved/loaded config.
  // Used to detect whether a provider switch requires a full re-index.
  const savedEmbedModelRef = useRef<string>('');

  // ── Load initial data ──────────────────────────────────────────────────────

  // biome-ignore lint/correctness/useExhaustiveDependencies: load on mount only
  useEffect(() => {
    void loadAll();
    // Subscribe to download progress events
    const unlistenDownload = listen<ModelDownloadProgress>('model-download-progress', (event) => {
      const progress = event.payload;
      if (!progress?.modelId || !progress?.status) return;
      // Surface terminal failures. Without this, a download that errors
      // immediately (404, SHA mismatch, network) is silently removed from
      // the active map and the Download button looks like it did nothing.
      if (progress.status === 'error') {
        const detail = progress.error?.trim() || 'Unknown error';
        setError(t('settings:ai.downloadFailedFor', { model: progress.modelId, detail }));
        addLog('error', 'ai', t('settings:ai.downloadFailedLog', { model: progress.modelId, detail }));
      }
      setDownloads((prev) => {
        if (progress.status === 'complete' || progress.status === 'error' || progress.status === 'cancelled') {
          // Remove from active downloads map. Cancelled leaves a `.partial`
          // file behind so a follow-up Download button click resumes via the
          // HTTP Range header in the backend.
          const next = { ...prev };
          delete next[progress.modelId];
          if (progress.status === 'complete') {
            // Reload full config so backend auto-selections are picked up
            if (catalogRefreshRef.current) clearTimeout(catalogRefreshRef.current);
            catalogRefreshRef.current = setTimeout(() => void loadAll(), 500);
          }
          return next;
        }
        return { ...prev, [progress.modelId]: progress };
      });
    });
    const unlistenConfigUpdate = listen('ai-config-updated', () => {
      void loadAll();
    });

    return () => {
      void unlistenDownload.then((u) => u());
      void unlistenConfigUpdate.then((u) => u());
      if (catalogRefreshRef.current) clearTimeout(catalogRefreshRef.current);
    };
  }, []);

  const loadCatalog = async () => {
    try {
      const models = await api.listCatalogModels();
      setCatalog(models);
    } catch {
      // Non-fatal — catalog might be unavailable on older builds
    }
  };

  const loadAll = async () => {
    setLoading(true);
    setError(null);
    try {
      const cfg = await api.getAiConfig();
      setConfig({
        provider: cfg.provider as AiConfigState['provider'],
        model: cfg.model,
        embeddingModel: cfg.embeddingModel,
        monthlyBudgetUsd: cfg.monthlyBudgetUsd,
        hasApiKey: cfg.hasApiKey,
        thinkingEnabled: cfg.thinkingEnabled,
      });
      savedEmbedModelRef.current = cfg.embeddingModel;

      await loadCatalog();

      // Ollama models (best-effort)
      try {
        const all = await api.listOllamaModels();
        setOllamaModels(all.filter((m) => !/(embed|nomic|bge|e5)/i.test(m)));
        const embeds = all.filter((m) => /(embed|nomic|bge|e5)/i.test(m));
        setOllamaEmbedModels(embeds.length > 0 ? embeds : ['nomic-embed-text']);
      } catch {
        setOllamaModels([]);
        setOllamaEmbedModels(['nomic-embed-text']);
      }

      // Routing mode preference
      try {
        const raw = await api.getPref('chat.routing_mode');
        setRoutingMode(isRoutingMode(raw) ? raw : DEFAULT_ROUTING_MODE);
      } catch {
        setRoutingMode(DEFAULT_ROUTING_MODE);
      }

      // AI output language preference. Reads the typed `ai_output_language_v2`
      // first; falls back to the legacy free-text `ai_output_language` so users
      // who configured a language before the v2 migration don't see a reset.
      // Unknown / unsupported values resolve to the "Same as UI" sentinel ("").
      try {
        const v2 = await api.getPref('ai_output_language_v2');
        if (v2 != null && /^(en|es|fr|de)$/i.test(v2.trim())) {
          setAiOutputLanguage(v2.trim().toLowerCase());
        } else {
          const legacy = await api.getPref('ai_output_language');
          const mapped: Record<string, string> = {
            english: 'en',
            spanish: 'es',
            french: 'fr',
            german: 'de',
          };
          const key = (legacy ?? '').trim().toLowerCase();
          setAiOutputLanguage(mapped[key] ?? '');
        }
      } catch {
        setAiOutputLanguage('');
      }

      // AI processing age cutoff (days) — default 365. 0 = no limit.
      try {
        const raw = await api.getPref('ai_max_email_age_days');
        if (raw != null && raw.trim() !== '') {
          const n = parseInt(raw, 10);
          setAiMaxEmailAgeDays(Number.isFinite(n) && n >= 0 ? n : 365);
        } else {
          setAiMaxEmailAgeDays(365);
        }
      } catch {
        setAiMaxEmailAgeDays(365);
      }

      // Keep-alive duration (seconds) for local models — default 30 min.
      try {
        const raw = await api.getPref('chat.keep_alive_seconds');
        if (raw != null && raw.trim() !== '') {
          const secs = parseInt(raw, 10);
          if (Number.isFinite(secs)) {
            setKeepAliveMinutes(secs < 0 ? -1 : Math.round(secs / 60));
          } else {
            setKeepAliveMinutes(30);
          }
        } else {
          setKeepAliveMinutes(30);
        }
      } catch {
        setKeepAliveMinutes(30);
      }

      // Context window (tokens) for the embedded model — default 8192. A stored
      // 0 means "auto"; surface it as the default so the input is never blank.
      try {
        const raw = await api.getPref('chat.n_ctx');
        if (raw != null && raw.trim() !== '') {
          const n = parseInt(raw, 10);
          setNCtx(Number.isFinite(n) && n >= 1024 ? n : 8192);
        } else {
          setNCtx(8192);
        }
      } catch {
        setNCtx(8192);
      }
    } catch (err) {
      setError(t('settings:ai.loadFailed', { error: errorText(err) }));
    } finally {
      setLoading(false);
    }
  };

  // ── Actions ────────────────────────────────────────────────────────────────

  const handleProviderChange = (p: AiConfigState['provider']) => {
    if (!config) return;
    setConfig({ ...config, provider: p });
    setError(null);
    setSuccess(null);
  };

  const handleSelectCatalogModel = (model: CatalogModel) => {
    if (!config) return;
    if (model.kind === 'chat') {
      setConfig({ ...config, model: model.id });
    } else {
      setConfig({ ...config, embeddingModel: model.id });
    }
  };

  const handleDownload = async (modelId: string) => {
    setError(null);
    try {
      await api.startModelDownload(modelId);
      addLog('info', 'ai', t('settings:ai.downloadStarted', { model: modelId }));
    } catch (err) {
      setError(t('settings:ai.modelDownloadFailed', { error: errorText(err) }));
      addLog('error', 'ai', t('settings:ai.downloadFailedGeneric', { error: errorText(err) }));
    }
  };

  const handleCancel = async (modelId: string) => {
    try {
      await api.cancelModelDownload(modelId);
    } catch {
      // Ignore cancel errors
    }
  };

  const handleDelete = async (model: CatalogModel) => {
    setError(null);
    try {
      await api.deleteLocalModel(model.id, model.kind);
      addLog('info', 'ai', t('settings:ai.deletedModel', { model: model.displayName }));
      await loadCatalog();
      // If the deleted model was selected, clear the selection
      if (config) {
        if (model.kind === 'chat' && config.model === model.id) {
          setConfig({ ...config, model: '' });
        } else if (model.kind === 'embedding' && config.embeddingModel === model.id) {
          setConfig({ ...config, embeddingModel: '' });
        }
      }
    } catch (err) {
      setError(t('settings:ai.deleteFailed', { error: errorText(err) }));
    }
  };

  const handleRoutingModeChange = async (mode: RoutingMode) => {
    setRoutingMode(mode);
    try {
      await api.setPref('chat.routing_mode', mode);
    } catch (err) {
      setError(t('settings:ai.routingSaveFailed', { error: errorText(err) }));
    }
  };

  const handleSave = async () => {
    if (!config) return;
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      const prevEmbedModel = savedEmbedModelRef.current;
      const wantsApiKey = config.provider === 'openrouter';
      const key = wantsApiKey && apiKey ? apiKey : null;

      await api.setAiConfig(
        config.provider,
        config.model,
        config.embeddingModel,
        key,
        config.monthlyBudgetUsd,
        config.thinkingEnabled,
      );
      savedEmbedModelRef.current = config.embeddingModel;

      // Save AI output language. We write the typed v2 key; the empty string
      // is the "Same as UI" sentinel and is accepted by the preferences
      // validator. The legacy `ai_output_language` is cleared on the same
      // write so it can no longer shadow the v2 value on next read.
      try {
        await api.setPref('ai_output_language_v2', aiOutputLanguage);
        await api.setPref('ai_output_language', '');
      } catch (err) {
        addLog('error', 'ai', t('settings:ai.outputLanguageSaveFailed', { error: errorText(err) }));
      }

      // Save keep-alive preference (-1 = pin forever, otherwise minutes→seconds).
      try {
        const secs = keepAliveMinutes < 0 ? -1 : Math.max(0, Math.round(keepAliveMinutes * 60));
        await api.setPref('chat.keep_alive_seconds', String(secs));
      } catch (err) {
        addLog('error', 'ai', t('settings:ai.keepAliveSaveFailed', { error: errorText(err) }));
      }

      // Save AI processing age cutoff (days). 0 = no limit; clamp negatives.
      try {
        const days = Number.isFinite(aiMaxEmailAgeDays) ? Math.max(0, Math.round(aiMaxEmailAgeDays)) : 365;
        await api.setPref('ai_max_email_age_days', String(days));
      } catch (err) {
        addLog('error', 'ai', t('settings:ai.ageCutoffSaveFailed', { error: errorText(err) }));
      }

      // Save context window (tokens) for the embedded model. Clamp to the
      // backend-accepted floor so the validator never rejects the write; the
      // per-model upper clamp happens at actor-spawn time.
      if (config.provider === 'llamacpp') {
        try {
          const tokens = Number.isFinite(nCtx) ? Math.max(1024, Math.round(nCtx)) : 8192;
          await api.setPref('chat.n_ctx', String(tokens));
        } catch (err) {
          addLog('error', 'ai', t('settings:ai.contextWindowSaveFailed', { error: errorText(err) }));
        }
      }

      // Trigger full re-index if the embedding model changed.
      const embedChanged = prevEmbedModel !== '' && prevEmbedModel !== config.embeddingModel;
      if (embedChanged) {
        addLog('info', 'ai', t('settings:ai.reindexStarting'));
        try {
          await api.regenerateEmbeddings(); // no accountId → all accounts
        } catch (err) {
          addLog('error', 'ai', t('settings:ai.reindexFailed', { error: errorText(err) }));
        }
      }

      setSuccess(embedChanged ? t('settings:ai.saveSuccessReindex') : t('settings:ai.saveSuccess'));
      addLog('success', 'ai', t('settings:ai.backendSet', { provider: config.provider, model: config.model }));
      if (apiKey) setApiKey('');
      void loadAll();
    } catch (err) {
      setError(t('settings:ai.saveFailed', { error: errorText(err) }));
    } finally {
      setSaving(false);
    }
  };

  const handleTest = async () => {
    if (!config) return;
    setTesting(true);
    setError(null);
    setSuccess(null);
    try {
      const wantsApiKey = config.provider === 'openrouter';
      const key = wantsApiKey && apiKey ? apiKey : null;
      const result = await api.testAiProvider(config.provider, config.model, key);
      setSuccess(t('settings:ai.testPassed', { result: result.substring(0, 120) }));
    } catch (err) {
      setError(t('settings:ai.testFailed', { error: errorText(err) }));
    } finally {
      setTesting(false);
    }
  };

  if (loading && !config) {
    return (
      <Shell embedded={embedded}>
        {!embedded && (
          <div className="flex items-center justify-between px-6 py-4 border-b border-gray-700">
            <h2 className="text-lg font-semibold text-gray-100">{t('settings:ai.title')}</h2>
            <CloseButton onClose={onClose} />
          </div>
        )}
        <p className="text-gray-400 text-sm p-6">{t('common:state.loading')}</p>
      </Shell>
    );
  }

  if (!config) {
    return (
      <Shell embedded={embedded}>
        {!embedded && (
          <div className="flex items-center justify-between px-6 py-4 border-b border-gray-700">
            <h2 className="text-lg font-semibold text-gray-100">{t('settings:ai.title')}</h2>
            <CloseButton onClose={onClose} />
          </div>
        )}
        <div className="p-6">
          {error && (
            <div className="mb-4 p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>
          )}
          <button
            onClick={loadAll}
            className="px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500"
          >
            {t('common:actions.retry')}
          </button>
        </div>
      </Shell>
    );
  }

  return (
    <>
      <Shell embedded={embedded}>
        {!embedded && (
          /* Header */
          <div className="flex items-center justify-between px-6 py-4 border-b border-gray-700 flex-shrink-0">
            <h2 className="text-lg font-semibold text-gray-100">{t('settings:ai.title')}</h2>
            <CloseButton onClose={onClose} />
          </div>
        )}

        {/* Scrollable body */}
        <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
          {error && <div className="p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>}
          {success && (
            <div className="p-3 bg-green-900/30 border border-green-800 rounded text-green-300 text-sm">{success}</div>
          )}

          {/* ── Master AI toggle ────────────────────────────────────────────── */}
          <section className="p-4 rounded-lg border border-gray-700 bg-[#2a2a2b]">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0">
                <h3 className="text-sm font-semibold text-gray-100">{t('settings:ai.features')}</h3>
                <p className="text-xs text-gray-400 mt-1">{t('settings:ai.featuresHelp')}</p>
              </div>
              <button
                type="button"
                onClick={() => {
                  if (aiEnabled) {
                    setConfirmDisable(true);
                  } else {
                    void setAiEnabled(true);
                  }
                }}
                aria-pressed={aiEnabled}
                className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                  aiEnabled ? 'bg-primary-600' : 'bg-gray-600'
                }`}
              >
                <span
                  className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                    aiEnabled ? 'translate-x-5' : 'translate-x-0'
                  }`}
                />
              </button>
            </div>
          </section>

          {/* When the master switch is off, hide every AI-specific control
              below. The toggle above stays visible so the user can re-enable. */}
          {aiEnabled && (
            <>
              {/* ── Backend selector ────────────────────────────────────────────── */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-2">{t('settings:ai.backend')}</label>
                <div className="flex gap-2">
                  <ProviderTab
                    active={config.provider === 'llamacpp'}
                    label={t('settings:ai.providerEmbeddedLabel')}
                    description={t('settings:ai.providerEmbeddedDesc')}
                    onClick={() => handleProviderChange('llamacpp')}
                  />
                  <ProviderTab
                    active={config.provider === 'ollama'}
                    label={t('settings:ai.providerOllamaLabel')}
                    description={t('settings:ai.providerOllamaDesc')}
                    onClick={() => handleProviderChange('ollama')}
                  />
                  <ProviderTab
                    active={config.provider === 'openrouter'}
                    label={t('settings:ai.providerOpenRouterLabel')}
                    description={t('settings:ai.providerOpenRouterDesc')}
                    onClick={() => handleProviderChange('openrouter')}
                  />
                </div>
              </div>

              {config.provider === 'llamacpp' && (
                <EmbeddedPanel
                  config={config}
                  setConfig={setConfig}
                  catalog={catalog}
                  downloads={downloads}
                  onSelectModel={handleSelectCatalogModel}
                  onDownload={(id) => void handleDownload(id)}
                  onCancel={(id) => void handleCancel(id)}
                  onDelete={(m) => void handleDelete(m)}
                />
              )}

              {config.provider === 'ollama' && (
                <OllamaPanel
                  config={config}
                  setConfig={setConfig}
                  ollamaModels={ollamaModels}
                  ollamaEmbedModels={ollamaEmbedModels}
                />
              )}

              {config.provider === 'openrouter' && (
                <OpenRouterPanel config={config} setConfig={setConfig} apiKey={apiKey} setApiKey={setApiKey} />
              )}

              {/* ── Shared preferences (routing, keep-alive, age cutoff, language) ── */}
              <AiSharedPreferences
                routingMode={routingMode}
                onRoutingModeChange={(mode) => void handleRoutingModeChange(mode)}
                keepAliveMinutes={keepAliveMinutes}
                onKeepAliveChange={setKeepAliveMinutes}
                aiMaxEmailAgeDays={aiMaxEmailAgeDays}
                onMaxEmailAgeDaysChange={setAiMaxEmailAgeDays}
                nCtx={nCtx}
                onNCtxChange={setNCtx}
                showContextWindow={config.provider === 'llamacpp'}
                aiOutputLanguage={aiOutputLanguage}
                onOutputLanguageChange={setAiOutputLanguage}
              />

              {/* ── Chat prompts (system + advanced retrieval prompts) ──────────── */}
              <ChatPromptsSection />
            </>
          )}
        </div>

        {/* Footer — only meaningful when AI is on. Save/Test write provider
            config that has no effect while the master switch is off. */}
        {aiEnabled && (
          <div className="px-6 py-4 border-t border-gray-700 flex gap-2 flex-shrink-0">
            <button
              onClick={handleTest}
              disabled={testing || !config.model}
              className="px-4 py-2 bg-gray-700 text-gray-200 rounded text-sm hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {testing ? t('settings:ai.testing') : t('settings:ai.test')}
            </button>
            <button
              onClick={handleSave}
              disabled={saving}
              className="flex-1 px-4 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50"
            >
              {saving ? t('common:state.saving') : t('common:actions.save')}
            </button>
          </div>
        )}
      </Shell>
      {confirmDisable && (
        <ConfirmDisableDialog
          onCancel={() => setConfirmDisable(false)}
          onConfirm={async () => {
            try {
              await setAiEnabled(false);
              addLog('info', 'ai', t('settings:ai.disabledLog'));
            } catch (err) {
              addLog('error', 'ai', t('settings:ai.disableFailed', { error: errorText(err) }));
            } finally {
              setConfirmDisable(false);
            }
          }}
        />
      )}
    </>
  );
}
