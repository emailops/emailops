import { listen } from '@tauri-apps/api/event';
import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { getSafeExternalUrl } from '@/lib/emailFormatting';
import { errorText } from '@/lib/errors';
import { credentialStoreKey } from '@/lib/platform';
import { useLogStore } from '@/stores/logStore';
import type { CatalogModel, ModelDownloadProgress } from '@/types';

const HUGGINGFACE_URL = 'https://huggingface.co';

type Backend = 'llamacpp' | 'ollama' | 'openrouter';

const OPENROUTER_DEFAULT_CHAT = 'openai/gpt-4o-mini';
const OPENROUTER_DEFAULT_EMBED = 'openai/text-embedding-3-small';

export function StepAiBackend({ onBack, onNext }: { onBack: () => void; onNext: () => void }) {
  const { t } = useTranslation(['auth']);
  const addLog = useLogStore((s) => s.addLog);
  const [backend, setBackend] = useState<Backend>('llamacpp');
  // See AiSettings: false on builds without llama.cpp and on Intel Macs, whose
  // GPU cannot run the Metal kernels. Null until probed — the card stays
  // enabled meanwhile rather than flickering disabled.
  const [embeddedAvailable, setEmbeddedAvailable] = useState<boolean | null>(null);
  const [catalog, setCatalog] = useState<CatalogModel[]>([]);
  const [downloads, setDownloads] = useState<Record<string, ModelDownloadProgress>>({});
  const [chatModelId, setChatModelId] = useState<string>('');
  const [embedModelId, setEmbedModelId] = useState<string>('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [orApiKey, setOrApiKey] = useState<string>('');
  const [orHasSavedKey, setOrHasSavedKey] = useState<boolean>(false);
  const [orChatModel, setOrChatModel] = useState<string>(OPENROUTER_DEFAULT_CHAT);
  const [orEmbedModel, setOrEmbedModel] = useState<string>(OPENROUTER_DEFAULT_EMBED);
  const [testStatus, setTestStatus] = useState<null | 'ok' | 'fail'>(null);
  const [testError, setTestError] = useState<string | null>(null);
  const [testing, setTesting] = useState(false);
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Model ids currently being linked (vs. downloaded) — lets the shared
  // model-download-progress event handler pick the right error message.
  // A ref (not state) because it's mutated inside the `listen` callback,
  // which is set up once and would otherwise close over a stale value.
  const linkingIds = useRef<Set<string>>(new Set());

  useEffect(() => {
    void (async () => {
      // Probe first: the embedded default below is only right where the runtime
      // can actually run. On an Intel Mac it cannot, and pre-selecting it walks
      // the user into downloading a multi-GB model that fails on first use.
      let embeddedOk: boolean | null = null;
      try {
        embeddedOk = (await api.detectAiCapability()).embeddedAiAvailable;
        setEmbeddedAvailable(embeddedOk);
        if (embeddedOk === false) setBackend('openrouter');
      } catch {
        // Probe failed — leave the card enabled and let the backend guard speak.
      }
      try {
        const cfg = await api.getAiConfig();
        // A stored `llamacpp` choice must not resurrect the unusable option:
        // this is exactly the state an already-affected install is in.
        if (
          (cfg.provider === 'llamacpp' && embeddedOk !== false) ||
          cfg.provider === 'ollama' ||
          cfg.provider === 'openrouter'
        ) {
          setBackend(cfg.provider);
        }
        if (cfg.provider === 'openrouter') {
          setOrChatModel(cfg.model || OPENROUTER_DEFAULT_CHAT);
          setOrEmbedModel(cfg.embeddingModel || OPENROUTER_DEFAULT_EMBED);
          setOrHasSavedKey(!!cfg.hasApiKey);
        } else {
          setChatModelId(cfg.model || '');
          setEmbedModelId(cfg.embeddingModel || '');
        }
      } catch {
        // Non-fatal — first-run defaults stand.
      }
      try {
        const cat = await api.listCatalogModels();
        setCatalog(cat);
        setChatModelId((prev) => prev || cat.find((m) => m.kind === 'chat' && m.recommended)?.id || '');
        setEmbedModelId((prev) => prev || cat.find((m) => m.kind === 'embedding' && m.recommended)?.id || '');
      } catch {
        setCatalog([]);
      }
    })();

    const unlisten = listen<ModelDownloadProgress>('model-download-progress', (event) => {
      const progress = event.payload;
      if (!progress?.modelId || !progress?.status) return;
      const isLinking = linkingIds.current.has(progress.modelId);
      if (progress.status === 'error') {
        const detail = progress.error?.trim() || t('auth:onboarding.aiBackend.unknownError');
        setError(
          isLinking
            ? t('auth:onboarding.aiBackend.linkFailedFor', { modelId: progress.modelId, detail })
            : t('auth:onboarding.aiBackend.downloadFailedFor', { modelId: progress.modelId, detail }),
        );
        addLog('error', 'ai', `Model ${isLinking ? 'link' : 'download'} failed (${progress.modelId}): ${detail}`);
      }
      setDownloads((prev) => {
        if (progress.status === 'complete' || progress.status === 'error' || progress.status === 'cancelled') {
          linkingIds.current.delete(progress.modelId);
          const next = { ...prev };
          delete next[progress.modelId];
          if (progress.status === 'complete') {
            if (refreshTimer.current) clearTimeout(refreshTimer.current);
            refreshTimer.current = setTimeout(() => {
              void api
                .listCatalogModels()
                .then((cat) => {
                  setCatalog(cat);
                  // The currently-selected id may still be the initial
                  // recommended-model guess, which is never downloaded when
                  // the user downloads/links a different model first. If
                  // the current pick isn't actually local, adopt whichever
                  // model just finished — mirrors the backend's own
                  // auto-select-when-missing logic (finish_model_op) so
                  // Continue submits a model that actually exists on disk.
                  const completed = cat.find((m) => m.id === progress.modelId);
                  if (!completed) return;
                  if (completed.kind === 'chat') {
                    setChatModelId((prev) =>
                      cat.some((m) => m.id === prev && m.kind === 'chat' && m.isLocal) ? prev : completed.id,
                    );
                  } else {
                    setEmbedModelId((prev) =>
                      cat.some((m) => m.id === prev && m.kind === 'embedding' && m.isLocal) ? prev : completed.id,
                    );
                  }
                })
                .catch(() => {});
            }, 400);
          }
          return next;
        }
        return { ...prev, [progress.modelId]: progress };
      });
    });

    return () => {
      void unlisten.then((u) => u());
      if (refreshTimer.current) clearTimeout(refreshTimer.current);
    };
  }, [addLog, t]);

  const handleDownload = async (modelId: string) => {
    setError(null);
    try {
      await api.startModelDownload(modelId);
      addLog('info', 'ai', `Started download: ${modelId}`);
    } catch (err) {
      const msg = errorText(err);
      setError(t('auth:onboarding.aiBackend.downloadFailed', { error: msg }));
      addLog('error', 'ai', `Model download failed: ${msg}`);
    }
  };

  const handleUseExistingFile = async (modelId: string) => {
    setError(null);
    let sourcePath: string | null;
    try {
      sourcePath = await openFileDialog({
        multiple: false,
        filters: [{ name: 'GGUF', extensions: ['gguf'] }],
      });
    } catch (err) {
      addLog('error', 'ai', `Failed to open file picker: ${errorText(err)}`);
      return;
    }
    if (!sourcePath) return; // User cancelled the picker.
    linkingIds.current.add(modelId);
    try {
      await api.linkLocalModel(modelId, sourcePath);
      addLog('info', 'ai', `Using existing file for ${modelId}: ${sourcePath}`);
    } catch (err) {
      linkingIds.current.delete(modelId);
      const msg = errorText(err);
      setError(t('auth:onboarding.aiBackend.linkFailed', { error: msg }));
      addLog('error', 'ai', `Failed to use existing file for ${modelId}: ${msg}`);
    }
  };

  const handleCancel = async (modelId: string) => {
    try {
      await api.cancelModelDownload(modelId);
    } catch {
      // Ignore — cancellation is best-effort.
    }
  };

  const chatModels = catalog.filter((m) => m.kind === 'chat');
  const embedModels = catalog.filter((m) => m.kind === 'embedding');
  const hasLocalEmbed = embedModels.some((m) => m.isLocal);
  const hasLocalChat = chatModels.some((m) => m.isLocal);

  const invalidateTest = () => {
    setTestStatus(null);
    setTestError(null);
  };

  const orHasKey = orHasSavedKey || orApiKey.trim().length > 0;
  const canContinue =
    backend === 'llamacpp'
      ? hasLocalEmbed && hasLocalChat
      : backend === 'openrouter'
        ? orHasKey && orChatModel.trim() !== '' && orEmbedModel.trim() !== ''
        : true;

  const handleTestConnection = async () => {
    if (testing) return;
    setTesting(true);
    setTestStatus(null);
    setTestError(null);
    try {
      const apiKeyArg = orApiKey.trim() === '' ? null : orApiKey.trim();
      await api.testAiProvider('openrouter', orChatModel.trim(), apiKeyArg);
      setTestStatus('ok');
      addLog('success', 'ai', `OpenRouter test OK (${orChatModel.trim()})`);
    } catch (err) {
      const msg = errorText(err);
      setTestStatus('fail');
      setTestError(msg);
      addLog('error', 'ai', `OpenRouter test failed: ${msg}`);
    } finally {
      setTesting(false);
    }
  };

  const handleContinue = async () => {
    setBusy(true);
    setError(null);
    try {
      let model = chatModelId;
      let embedding = embedModelId;
      let apiKey: string | null = null;
      if (backend === 'llamacpp') {
        if (!model) model = chatModels.find((m) => m.isLocal)?.id || '';
        if (!embedding) embedding = embedModels.find((m) => m.isLocal)?.id || '';
      } else if (backend === 'openrouter') {
        model = orChatModel.trim();
        embedding = orEmbedModel.trim();
        apiKey = orApiKey.trim() === '' ? null : orApiKey.trim();
      }
      await api.setAiConfig(backend, model, embedding, apiKey, 0, false);
      addLog('success', 'ai', `AI backend set to ${backend}`);
      onNext();
    } catch (err) {
      const msg = errorText(err);
      setError(t('auth:onboarding.aiBackend.saveConfigFailed', { error: msg }));
    } finally {
      setBusy(false);
    }
  };

  const handleOpenHuggingFace = () => {
    const safe = getSafeExternalUrl(HUGGINGFACE_URL);
    if (!safe) return;
    void openExternal(safe).catch((err) => {
      addLog('error', 'ai', `Failed to open ${safe}: ${errorText(err)}`);
    });
  };

  return (
    <div className="space-y-5">
      <div className="space-y-2">
        <p className="text-sm text-gray-200 font-medium">{t('auth:onboarding.aiBackend.introPrivacy')}</p>
        <p className="text-xs text-gray-400">{t('auth:onboarding.aiBackend.introModels')}</p>
      </div>

      <div className="grid grid-cols-3 gap-2">
        <BackendCard
          selected={backend === 'llamacpp'}
          onSelect={() => setBackend('llamacpp')}
          title={t('auth:onboarding.aiBackend.embeddedTitle')}
          subtitle={t('auth:onboarding.aiBackend.embeddedSubtitle')}
          detail={t('auth:onboarding.aiBackend.embeddedDetail')}
          disabled={embeddedAvailable === false}
          disabledReason={t('auth:onboarding.aiBackend.embeddedUnavailable')}
        />
        <BackendCard
          selected={backend === 'ollama'}
          onSelect={() => setBackend('ollama')}
          title={t('auth:onboarding.aiBackend.ollamaTitle')}
          subtitle={t('auth:onboarding.aiBackend.ollamaSubtitle')}
          detail={t('auth:onboarding.aiBackend.ollamaDetail')}
        />
        <BackendCard
          selected={backend === 'openrouter'}
          onSelect={() => {
            setBackend('openrouter');
            invalidateTest();
          }}
          title={t('auth:onboarding.aiBackend.openrouterTitle')}
          subtitle={t('auth:onboarding.aiBackend.openrouterSubtitle')}
          detail={t('auth:onboarding.aiBackend.openrouterDetail')}
        />
      </div>

      {backend === 'llamacpp' && (
        <div className="space-y-4">
          <div className="space-y-1">
            <p className="text-sm text-gray-300">
              {t('auth:onboarding.aiBackend.modelsSourcePrefix')}{' '}
              <button
                type="button"
                onClick={handleOpenHuggingFace}
                className="text-primary-400 hover:text-primary-300 underline underline-offset-2 transition-colors"
              >
                {t('auth:onboarding.aiBackend.modelsSourceLink')}
              </button>
              {t('auth:onboarding.aiBackend.modelsSourceSuffix')}
            </p>
            <p className="text-xs text-gray-500">{t('auth:onboarding.aiBackend.embeddingBundled')}</p>
          </div>

          <div>
            <div className="flex items-baseline justify-between mb-1.5">
              <h3 className="text-sm font-semibold text-gray-200">{t('auth:onboarding.aiBackend.chatModel')}</h3>
              <span className={`text-xs ${hasLocalChat ? 'text-green-400' : 'text-amber-400'}`}>
                {hasLocalChat
                  ? t('auth:onboarding.aiBackend.ready')
                  : t('auth:onboarding.aiBackend.requiredToContinue')}
              </span>
            </div>
            <p className="text-xs text-gray-500 mb-2">{t('auth:onboarding.aiBackend.chatHelp')}</p>
            <div className="space-y-1.5">
              {chatModels.length === 0 && (
                <div className="text-xs text-gray-500 italic">{t('auth:onboarding.aiBackend.noCatalog')}</div>
              )}
              {chatModels.map((m) => (
                <CompactModelRow
                  key={m.id}
                  model={m}
                  isSelected={chatModelId === m.id}
                  progress={downloads[m.id] ?? null}
                  onSelect={() => setChatModelId(m.id)}
                  onDownload={() => void handleDownload(m.id)}
                  onUseExistingFile={() => void handleUseExistingFile(m.id)}
                  onCancel={() => void handleCancel(m.id)}
                />
              ))}
            </div>
          </div>
        </div>
      )}

      {backend === 'ollama' && (
        <div className="p-3 bg-[#27272a] border border-gray-700 rounded text-xs text-gray-400">
          {t('auth:onboarding.aiBackend.ollamaFinishHint')}{' '}
          <span className="text-gray-200 font-medium">{t('auth:onboarding.aiBackend.ollamaFinishHintOllama')}</span>{' '}
          {t('auth:onboarding.aiBackend.ollamaFinishHintRest')}
        </div>
      )}

      {backend === 'openrouter' && (
        <div className="space-y-3">
          <div>
            <label className="block text-xs font-medium text-gray-300 mb-1">
              {t('auth:onboarding.aiBackend.apiKey')}
              {orHasSavedKey && (
                <span className="text-gray-500 font-normal"> {t('auth:onboarding.aiBackend.apiKeySaved')}</span>
              )}
            </label>
            <input
              type="password"
              value={orApiKey}
              onChange={(e) => {
                setOrApiKey(e.target.value);
                invalidateTest();
              }}
              placeholder={
                orHasSavedKey
                  ? t('auth:onboarding.aiBackend.apiKeyPlaceholderSaved')
                  : t('auth:onboarding.aiBackend.apiKeyPlaceholder')
              }
              className="w-full bg-[#27272a] text-gray-200 border border-gray-700 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
            />
            <p className="text-[11px] text-gray-500 mt-1">
              {t('auth:onboarding.aiBackend.apiKeyHelpPart1', {
                store: t(credentialStoreKey(api.currentPlatform())),
              })}{' '}
              <span className="text-gray-400">{t('auth:onboarding.aiBackend.apiKeyHelpUrl')}</span>
              {t('auth:onboarding.aiBackend.apiKeyHelpDot')}
            </p>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
            <div>
              <label className="block text-xs font-medium text-gray-300 mb-1">
                {t('auth:onboarding.aiBackend.chatModel')}
              </label>
              <input
                type="text"
                value={orChatModel}
                onChange={(e) => {
                  setOrChatModel(e.target.value);
                  invalidateTest();
                }}
                placeholder={OPENROUTER_DEFAULT_CHAT}
                className="w-full bg-[#27272a] text-gray-200 border border-gray-700 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
              />
            </div>
            <div>
              <label className="block text-xs font-medium text-gray-300 mb-1">
                {t('auth:onboarding.aiBackend.embeddingModel')}
              </label>
              <input
                type="text"
                value={orEmbedModel}
                onChange={(e) => {
                  setOrEmbedModel(e.target.value);
                  invalidateTest();
                }}
                placeholder={OPENROUTER_DEFAULT_EMBED}
                className="w-full bg-[#27272a] text-gray-200 border border-gray-700 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none font-mono"
              />
            </div>
          </div>

          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={() => void handleTestConnection()}
              disabled={testing || !orHasKey || orChatModel.trim() === ''}
              className="px-3 py-1.5 text-xs bg-gray-700 hover:bg-gray-600 text-gray-100 rounded disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            >
              {testing ? t('auth:onboarding.aiBackend.testing') : t('auth:onboarding.aiBackend.testConnection')}
            </button>
            {testStatus === 'ok' && (
              <span className="text-xs text-green-400">{t('auth:onboarding.aiBackend.testOk')}</span>
            )}
            {testStatus === 'fail' && (
              <span className="text-xs text-red-400 truncate" title={testError ?? undefined}>
                ✗ {testError || t('auth:onboarding.aiBackend.testFailFallback')}
              </span>
            )}
            {testStatus === null && !testing && (
              <span className="text-[11px] text-gray-500">{t('auth:onboarding.aiBackend.testOptional')}</span>
            )}
          </div>
        </div>
      )}

      {error && <div className="p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>}

      <div className="flex justify-between">
        <button
          onClick={onBack}
          className="px-4 py-2 text-sm text-gray-400 hover:text-gray-200 hover:bg-gray-800 rounded transition-colors"
        >
          {t('auth:onboarding.aiBackend.back')}
        </button>
        <button
          onClick={() => void handleContinue()}
          disabled={!canContinue || busy}
          className="px-5 py-2 bg-primary-600 text-white rounded text-sm hover:bg-primary-500 disabled:opacity-50 disabled:cursor-not-allowed"
          title={
            !canContinue
              ? backend === 'openrouter'
                ? t('auth:onboarding.aiBackend.continueTitleOpenrouter')
                : t('auth:onboarding.aiBackend.continueTitleChat')
              : undefined
          }
        >
          {busy ? t('auth:onboarding.aiBackend.saving') : t('auth:onboarding.aiBackend.continue')}
        </button>
      </div>
    </div>
  );
}

function BackendCard({
  selected,
  onSelect,
  title,
  subtitle,
  detail,
  disabled,
  disabledReason,
}: {
  selected: boolean;
  onSelect: () => void;
  title: string;
  subtitle: string;
  detail: string;
  /** Inert + greyed out; see ProviderTab for why the embedded runtime needs this. */
  disabled?: boolean;
  disabledReason?: string;
}) {
  return (
    <button
      type="button"
      onClick={disabled ? undefined : onSelect}
      disabled={disabled}
      title={disabled ? disabledReason : undefined}
      className={`text-left p-3 rounded-lg border-2 transition-colors ${
        disabled
          ? 'border-gray-800 bg-[#212123] cursor-not-allowed opacity-60'
          : selected
            ? 'border-primary-500 bg-primary-900/15'
            : 'border-gray-700 bg-[#27272a] hover:border-gray-500'
      }`}
    >
      <div className={`text-sm font-semibold ${disabled ? 'text-gray-500' : 'text-gray-100'}`}>{title}</div>
      <div className={`text-xs mt-0.5 ${selected && !disabled ? 'text-primary-300' : 'text-gray-500'}`}>{subtitle}</div>
      <div className="text-xs text-gray-500 mt-1">{disabled ? (disabledReason ?? detail) : detail}</div>
    </button>
  );
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const gb = bytes / 1e9;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / 1e6;
  return `${mb.toFixed(0)} MB`;
}

function CompactModelRow({
  model,
  isSelected,
  progress,
  onSelect,
  onDownload,
  onUseExistingFile,
  onCancel,
}: {
  model: CatalogModel;
  isSelected: boolean;
  progress: ModelDownloadProgress | null;
  onSelect: () => void;
  onDownload: () => void;
  onUseExistingFile: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation(['auth']);
  const isDownloading = progress !== null && progress.status === 'downloading';
  const isVerifying = progress?.status === 'verifying';
  const pct =
    progress && progress.totalBytes > 0 ? Math.round((progress.downloadedBytes / progress.totalBytes) * 100) : 0;

  return (
    <div
      className={`p-2.5 rounded border transition-colors ${
        isSelected ? 'border-primary-600 bg-primary-900/15' : 'border-gray-700 bg-[#2a2a2b]'
      }`}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="flex items-start gap-2 min-w-0 flex-1">
          {model.isLocal && (
            <button
              type="button"
              onClick={onSelect}
              className="mt-0.5 flex-shrink-0"
              title={t('auth:onboarding.aiBackend.useThisModel')}
            >
              <div
                className={`w-4 h-4 rounded-full border-2 flex items-center justify-center ${
                  isSelected ? 'border-primary-500' : 'border-gray-500'
                }`}
              >
                {isSelected && <div className="w-2 h-2 rounded-full bg-primary-500" />}
              </div>
            </button>
          )}
          <div className="min-w-0">
            <div className="flex items-center gap-1.5 flex-wrap">
              <span className="text-xs font-medium text-gray-200">{model.displayName}</span>
              {model.recommended && (
                <span className="text-[10px] px-1.5 py-0.5 bg-primary-900/50 text-primary-400 rounded border border-primary-800">
                  {t('auth:onboarding.aiBackend.recommendedBadge')}
                </span>
              )}
              {model.isLocal && (
                <span className="text-[10px] px-1.5 py-0.5 bg-green-900/40 text-green-400 rounded border border-green-800">
                  {model.isLinked
                    ? t('auth:onboarding.aiBackend.linkedBadge')
                    : t('auth:onboarding.aiBackend.downloadedBadge')}
                </span>
              )}
            </div>
            <div className="text-[11px] text-gray-500 mt-0.5">
              {t('auth:onboarding.aiBackend.modelMeta', {
                minRam: model.minRamGb,
                size: formatBytes(model.sizeBytes),
                license: model.license,
              })}
            </div>
          </div>
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          {model.isLocal ? null : isDownloading || isVerifying ? (
            <button
              type="button"
              onClick={onCancel}
              className="px-2 py-1 text-xs text-gray-400 hover:text-gray-200 hover:bg-gray-700 rounded transition-colors"
            >
              {t('auth:onboarding.aiBackend.cancel')}
            </button>
          ) : (
            <>
              <button
                type="button"
                onClick={onUseExistingFile}
                className="px-2 py-1 text-xs text-gray-300 hover:text-gray-100 hover:bg-gray-700 rounded border border-gray-600 transition-colors"
              >
                {t('auth:onboarding.aiBackend.useExistingFile')}
              </button>
              <button
                type="button"
                onClick={onDownload}
                className="px-2 py-1 text-xs bg-primary-700 hover:bg-primary-600 text-white rounded transition-colors"
              >
                {t('auth:onboarding.aiBackend.download')}
              </button>
            </>
          )}
        </div>
      </div>

      {(isDownloading || isVerifying) && progress && (
        <div className="mt-2">
          <div className="flex items-center justify-between text-[11px] text-gray-400 mb-1">
            <span>
              {isVerifying
                ? t('auth:onboarding.aiBackend.verifying')
                : `${formatBytes(progress.downloadedBytes)} / ${formatBytes(progress.totalBytes)}`}
            </span>
            {isDownloading && <span>{pct}%</span>}
          </div>
          <div className="h-1 bg-gray-700 rounded-full overflow-hidden">
            <div
              className={`h-full rounded-full transition-all ${isVerifying ? 'bg-yellow-500' : 'bg-primary-500'}`}
              style={{ width: isVerifying ? '100%' : `${pct}%` }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
