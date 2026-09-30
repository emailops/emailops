import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText, isDataPolicyError } from '@/lib/errors';
import { useAiStore } from '@/stores/aiStore';
import { useHelpDocsEnabledStore } from '@/stores/featureToggleStore';
import { useLogStore } from '@/stores/logStore';
import type { AiModelInfo, CatalogModel, ModelDownloadProgress } from '@/types';
import { AiSharedPreferences } from './AiSettings/AiSharedPreferences';
import { ChatPromptsSection } from './AiSettings/ChatPromptsSection';
import { ConfirmDisableDialog } from './AiSettings/ConfirmDisableDialog';
import { ConfirmReindexDialog } from './AiSettings/ConfirmReindexDialog';
import { EmbeddedPanel } from './AiSettings/EmbeddedPanel';
import {
  chatModelForProvider,
  contextBudgetFromPref,
  contextBudgetToPref,
  DEFAULT_CONTEXT_BUDGET,
  embeddingModelChanged,
  embeddingModelForProvider,
  needsEmbeddingProbe,
} from './AiSettings/helpers';
import { OllamaPanel } from './AiSettings/OllamaPanel';
import { OpenRouterPanel } from './AiSettings/OpenRouterPanel';
import { ProviderTab } from './AiSettings/ProviderTab';
import { type AiConfigState, DEFAULT_ROUTING_MODE, isRoutingMode, type RoutingMode } from './AiSettings/types';
import { SettingsPanel } from './SettingsPanel';

/**
 * AI configuration screen. Owns provider selection, model + key state,
 * download orchestration, and assorted preferences (routing mode, keep-alive,
 * output language, chat prompts). Provider-specific UI lives in panel
 * sub-components under ./AiSettings/.
 */
export function AiSettings() {
  const { t } = useTranslation(['common', 'settings']);
  // Master AI enable/disable — drives whether any AI command runs and whether
  // AI surfaces show up in the UI. Stored in `user_preferences.ai_enabled`.
  const { enabled: aiEnabled, setEnabled: setAiEnabled } = useAiStore();
  const [confirmDisable, setConfirmDisable] = useState(false);
  // Save is waiting for the user to accept that the email index is rebuilt.
  const [confirmReindex, setConfirmReindex] = useState(false);
  const [config, setConfig] = useState<AiConfigState | null>(null);
  const [catalog, setCatalog] = useState<CatalogModel[]>([]);
  // Map modelId → in-progress download info
  const [downloads, setDownloads] = useState<Record<string, ModelDownloadProgress>>({});
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaEmbedModels, setOllamaEmbedModels] = useState<string[]>([]);
  const [openRouterEmbedModels, setOpenRouterEmbedModels] = useState<AiModelInfo[]>([]);
  const [apiKey, setApiKey] = useState('');
  const [routingMode, setRoutingMode] = useState<RoutingMode>(DEFAULT_ROUTING_MODE);
  const [aiOutputLanguage, setAiOutputLanguage] = useState<string>('Spanish');
  const { enabled: helpDocsEnabled, setEnabled: setHelpDocsEnabled } = useHelpDocsEnabledStore();
  // Minutes the local model is kept in RAM between chat turns. 0 = evict
  // immediately after use, -1 / empty-input = pin forever. Stored as seconds
  // in the `chat.keep_alive_seconds` preference.
  const [keepAliveMinutes, setKeepAliveMinutes] = useState<number>(30);
  // Cap on how far back AI processing (embeddings + classification) reaches.
  // An account with at most `ai_max_email_count` emails is processed whole;
  // a larger one only for the last `ai_max_email_age_days` days. 0 emails =
  // always apply the day limit, 0 days = no limit.
  const [aiMaxEmailCount, setAiMaxEmailCount] = useState<number>(1000);
  const [aiMaxEmailAgeDays, setAiMaxEmailAgeDays] = useState<number>(365);
  // Context window (tokens) for the embedded llama.cpp chat model. Stored in
  // `chat.n_ctx`; an unset pref (or stored 0 = auto) shows this machine's
  // RAM-tiered auto value (8192/16384/32768, via get_auto_n_ctx). The backend
  // clamps the saved value to [1024, model-trained-context].
  const [nCtx, setNCtx] = useState<number>(8192);
  // What the field showed right after load. Save skips the `chat.n_ctx` write
  // when the user never touched the field and no explicit pref existed, so
  // saving unrelated settings can't pin the machine's auto choice.
  const nCtxLoadedRef = useRef<{ explicit: boolean; value: number }>({ explicit: false, value: 8192 });
  // Prompt budget (tokens) for remote OpenRouter models, stored in
  // `chat.remote_n_ctx_budget`; unset shows the default. Saved only when the
  // user changed it, so the default is never pinned by an unrelated save.
  const [contextBudget, setContextBudget] = useState<number>(DEFAULT_CONTEXT_BUDGET);
  const contextBudgetLoadedRef = useRef<number>(DEFAULT_CONTEXT_BUDGET);
  // Whether the embedded runtime can actually run on this machine. False both
  // for builds compiled without llama.cpp and for Intel Macs, whose GPU cannot
  // execute the Metal kernels — selecting it there failed every turn with an
  // opaque `Decode Error -3`. Null until the probe returns; the tab stays
  // enabled meanwhile so a slow probe can't hide a working option.
  const [embeddedAvailable, setEmbeddedAvailable] = useState<boolean | null>(null);
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
  // The provider that was active when we last saved/loaded config: the saved
  // chat and embedding models only mean something to it (see
  // chatModelForProvider / embeddingModelForProvider).
  const savedProviderRef = useRef<string>('');
  const savedModelRef = useRef<string>('');

  // ── Load initial data ──────────────────────────────────────────────────────

  // biome-ignore lint/correctness/useExhaustiveDependencies: load on mount only
  useEffect(() => {
    void loadAll();
    // A failed probe must not disable a provider that may well work — leave the
    // tab enabled and let the backend's own guard report the real problem.
    api
      .detectAiCapability()
      .then((cap) => setEmbeddedAvailable(cap.embeddedAiAvailable))
      .catch(() => setEmbeddedAvailable(null));
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

  // OpenRouter's embedding models, once the OpenRouter tab is open and a key
  // is saved to ask with. A failed listing leaves the saved model selectable.
  const openRouterListable = config?.provider === 'openrouter' && config.hasApiKey;
  // biome-ignore lint/correctness/useExhaustiveDependencies: reload only when the tab or the saved key changes
  useEffect(() => {
    if (!openRouterListable) return;
    let stale = false;
    api
      .listAiEmbeddingModels('openrouter')
      .then((models) => {
        if (!stale) setOpenRouterEmbedModels(models);
      })
      .catch((err) => {
        if (!stale) addLog('error', 'ai', t('settings:openRouter.embeddingListFailed', { error: errorText(err) }));
      });
    return () => {
      stale = true;
    };
  }, [openRouterListable]);

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
        embeddingModelValidated: cfg.embeddingModelValidated,
        monthlyBudgetUsd: cfg.monthlyBudgetUsd,
        hasApiKey: cfg.hasApiKey,
        thinkingEnabled: cfg.thinkingEnabled,
        zeroDataRetention: cfg.zeroDataRetention,
      });
      savedEmbedModelRef.current = cfg.embeddingModel;
      savedProviderRef.current = cfg.provider;
      savedModelRef.current = cfg.model;

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

      // AI processing email-count limit — default 1000. 0 = always apply days.
      try {
        const raw = await api.getPref('ai_max_email_count');
        const n = raw != null && raw.trim() !== '' ? parseInt(raw, 10) : 1000;
        setAiMaxEmailCount(Number.isFinite(n) && n >= 0 ? n : 1000);
      } catch {
        setAiMaxEmailCount(1000);
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

      // Context window (tokens) for the embedded model. A stored 0 (or no
      // pref) means "auto" — surface the machine's RAM-tiered auto value so
      // the input is never blank and never suggests a downgrade.
      try {
        const autoNCtx = await api.getAutoNCtx().catch(() => 8192);
        const raw = await api.getPref('chat.n_ctx');
        const n = raw != null && raw.trim() !== '' ? parseInt(raw, 10) : Number.NaN;
        if (Number.isFinite(n) && n >= 1024) {
          nCtxLoadedRef.current = { explicit: true, value: n };
          setNCtx(n);
        } else {
          nCtxLoadedRef.current = { explicit: false, value: autoNCtx };
          setNCtx(autoNCtx);
        }
      } catch {
        nCtxLoadedRef.current = { explicit: false, value: 8192 };
        setNCtx(8192);
      }

      try {
        const budget = contextBudgetFromPref(await api.getPref('chat.remote_n_ctx_budget'));
        contextBudgetLoadedRef.current = budget;
        setContextBudget(budget);
      } catch (err) {
        addLog('error', 'ai', t('settings:openRouter.contextBudgetLoadFailed', { error: errorText(err) }));
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
    setConfig({
      ...config,
      provider: p,
      model: chatModelForProvider(
        p,
        { provider: savedProviderRef.current, model: savedModelRef.current },
        { catalog, ollamaModels },
      ),
      embeddingModel: embeddingModelForProvider(
        p,
        { provider: savedProviderRef.current, embeddingModel: savedEmbedModelRef.current },
        { catalog, ollamaEmbedModels },
      ),
    });
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

  const handleSave = () => {
    if (!config) return;
    setError(null);
    setSuccess(null);
    // OpenRouter has no model to fall back to: an empty id would be saved as
    // it is and every request would fail.
    if (config.provider === 'openrouter' && config.model.trim() === '') {
      setError(t('settings:openRouter.chatModelRequired'));
      return;
    }
    // A changed embedding model deletes and rebuilds the whole index: ask first.
    if (embeddingModelChanged(savedEmbedModelRef.current, config.embeddingModel)) {
      setConfirmReindex(true);
      return;
    }
    void save();
  };

  const save = async () => {
    if (!config) return;
    setSaving(true);
    try {
      const prevEmbedModel = savedEmbedModelRef.current;
      const wantsApiKey = config.provider === 'openrouter';
      const key = wantsApiKey && apiKey ? apiKey : null;

      // An OpenRouter embedding model must fit the email index before it is
      // saved: nothing is written when the check fails.
      if (needsEmbeddingProbe(config, prevEmbedModel)) {
        try {
          await api.validateOpenRouterEmbeddingModel(config.embeddingModel, key, config.zeroDataRetention);
        } catch (err) {
          setError(
            config.zeroDataRetention && isDataPolicyError(err)
              ? t('settings:openRouter.embeddingZdrBlocked', { model: config.embeddingModel })
              : t('settings:openRouter.embeddingCheckFailed', { error: errorText(err) }),
          );
          return;
        }
      }

      await api.setAiConfig(
        config.provider,
        config.model,
        config.embeddingModel,
        key,
        config.monthlyBudgetUsd,
        config.thinkingEnabled,
        config.zeroDataRetention,
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

      // Save AI processing email-count limit. Clamp negatives.
      try {
        const count = Number.isFinite(aiMaxEmailCount) ? Math.max(0, Math.round(aiMaxEmailCount)) : 1000;
        await api.setPref('ai_max_email_count', String(count));
      } catch (err) {
        addLog('error', 'ai', t('settings:ai.emailCountCutoffSaveFailed', { error: errorText(err) }));
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
      // per-model upper clamp happens at actor-spawn time. Skipped entirely
      // when the field still shows the untouched auto value — writing it
      // would pin this machine's RAM-tiered auto choice forever.
      const nCtxUntouchedAuto = !nCtxLoadedRef.current.explicit && nCtx === nCtxLoadedRef.current.value;
      if (config.provider === 'llamacpp' && !nCtxUntouchedAuto) {
        try {
          const tokens = Number.isFinite(nCtx) ? Math.max(1024, Math.round(nCtx)) : 8192;
          await api.setPref('chat.n_ctx', String(tokens));
          nCtxLoadedRef.current = { explicit: true, value: tokens };
        } catch (err) {
          addLog('error', 'ai', t('settings:ai.contextWindowSaveFailed', { error: errorText(err) }));
        }
      }

      if (config.provider === 'openrouter' && contextBudget !== contextBudgetLoadedRef.current) {
        try {
          const tokens = contextBudgetToPref(contextBudget);
          await api.setPref('chat.remote_n_ctx_budget', tokens);
          contextBudgetLoadedRef.current = Number(tokens);
          setContextBudget(Number(tokens));
        } catch (err) {
          addLog('error', 'ai', t('settings:openRouter.contextBudgetSaveFailed', { error: errorText(err) }));
        }
      }

      // Trigger full re-index if the embedding model changed.
      const embedChanged = embeddingModelChanged(prevEmbedModel, config.embeddingModel);
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
      <SettingsPanel>
        <p className="text-gray-400 text-sm p-6">{t('common:state.loading')}</p>
      </SettingsPanel>
    );
  }

  if (!config) {
    return (
      <SettingsPanel>
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
      </SettingsPanel>
    );
  }

  return (
    <>
      <SettingsPanel
        // Save/Test write provider config that has no effect while the
        // master switch is off, so the footer is only meaningful when AI is on.
        footer={
          aiEnabled && (
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
          )
        }
      >
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
                  disabled={embeddedAvailable === false}
                  disabledReason={t('settings:ai.providerEmbeddedUnavailable')}
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
              <OpenRouterPanel
                config={config}
                setConfig={setConfig}
                apiKey={apiKey}
                setApiKey={setApiKey}
                contextBudget={contextBudget}
                onContextBudgetChange={setContextBudget}
                embeddingModels={openRouterEmbedModels}
                embeddingNeedsCheck={needsEmbeddingProbe(config, savedEmbedModelRef.current)}
              />
            )}

            {/* ── Shared preferences (routing, keep-alive, age cutoff, language) ── */}
            <AiSharedPreferences
              routingMode={routingMode}
              onRoutingModeChange={(mode) => void handleRoutingModeChange(mode)}
              keepAliveMinutes={keepAliveMinutes}
              onKeepAliveChange={setKeepAliveMinutes}
              showKeepAlive={config.provider !== 'openrouter'}
              aiMaxEmailCount={aiMaxEmailCount}
              onMaxEmailCountChange={setAiMaxEmailCount}
              aiMaxEmailAgeDays={aiMaxEmailAgeDays}
              onMaxEmailAgeDaysChange={setAiMaxEmailAgeDays}
              nCtx={nCtx}
              onNCtxChange={setNCtx}
              showContextWindow={config.provider === 'llamacpp'}
              aiOutputLanguage={aiOutputLanguage}
              onOutputLanguageChange={setAiOutputLanguage}
              helpDocsEnabled={helpDocsEnabled}
              onHelpDocsEnabledChange={(v) => {
                setHelpDocsEnabled(v).catch((err) => addLog('error', 'ai', `Failed to save help_docs_enabled: ${err}`));
              }}
            />

            {/* ── Chat prompts (system + advanced retrieval prompts) ──────────── */}
            <ChatPromptsSection />
          </>
        )}
      </SettingsPanel>
      {confirmReindex && (
        <ConfirmReindexDialog
          provider={config.provider}
          embeddingModel={config.embeddingModel}
          onCancel={() => setConfirmReindex(false)}
          onConfirm={() => {
            setConfirmReindex(false);
            void save();
          }}
        />
      )}
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
