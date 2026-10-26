import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import type { ServerContextReport } from '@/lib/api';
import type { AiModelInfo, CatalogModel } from '@/types';
import { OpenRouterSpendControls } from './OpenRouterSpendControls';
import { openRouterEmbeddingSelectOptions } from './openRouterEmbeddingOptions';
import { contextReportLines } from './serverSettings';
import { ThinkingToggle } from './ThinkingToggle';
import type { AiConfigState, EmbeddingSource, ServerState } from './types';

interface ServerPanelProps {
  config: AiConfigState;
  server: ServerState;
  setConfig: (next: AiConfigState) => void;
  /** A new server key typed here; empty keeps the saved one. */
  apiKey: string;
  setApiKey: (key: string) => void;
  /** The saved key will be deleted on Save. */
  clearApiKey: boolean;
  setClearApiKey: (clear: boolean) => void;
  /** What the server listed at `GET {base}/models`, once asked. */
  serverModels: string[];
  onLoadModels: () => void;
  loadingModels: boolean;
  /** Whether the in-app embedding model can run here (null while unknown). */
  embeddedAvailable: boolean | null;
  catalog: CatalogModel[];
  /** OpenRouter's embedding models; empty until a saved key lets them load. */
  openRouterEmbeddingModels: AiModelInfo[];
  /** The OpenRouter embedding model will be checked on Save. */
  embeddingNeedsCheck: boolean;
  /** The context window the server reported for what is typed here; null
   *  until Test or Save asks, or once the server, model or window changes. */
  contextReport: ServerContextReport | null;
}

const INPUT_CLASS =
  'w-full bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none';

/** The embedding model to offer when the source changes: none for the server
 *  and OpenRouter (keyword search until one is chosen), the local in-app one. */
function embeddingModelForSource(source: EmbeddingSource, catalog: CatalogModel[]): string {
  if (source !== 'inapp') return '';
  const models = catalog.filter((m) => m.kind === 'embedding');
  return (models.find((m) => m.isLocal) ?? models.find((m) => m.recommended))?.id ?? '';
}

/**
 * OpenAI-compatible server panel — a server the user runs (LM Studio, vLLM,
 * llama.cpp `llama-server`, LiteLLM, a proxy): base URL, an optional key kept
 * in the keychain, the chat model from the server's list or typed in, the
 * context size and where the email index is computed. By default that is the
 * same server, so mail goes to one destination only.
 *
 * Plain `http://` is accepted for this machine only; a server elsewhere needs
 * `https://`, and saving one asks the user to confirm the disclosure first
 * (`ServerDisclosureDialog`).
 */
export function ServerPanel({
  config,
  server,
  setConfig,
  apiKey,
  setApiKey,
  clearApiKey,
  setClearApiKey,
  serverModels,
  onLoadModels,
  loadingModels,
  embeddedAvailable,
  catalog,
  openRouterEmbeddingModels,
  embeddingNeedsCheck,
  contextReport,
}: ServerPanelProps) {
  const { t } = useTranslation(['common', 'settings']);
  const setServer = (next: Partial<ServerState>) => setConfig({ ...config, server: { ...server, ...next } });
  const sourceOptions: { value: EmbeddingSource; label: string }[] = [
    { value: 'server', label: t('settings:server.embeddingSourceServer') },
    ...(embeddedAvailable === false
      ? []
      : [{ value: 'inapp' as const, label: t('settings:server.embeddingSourceInApp') }]),
    { value: 'openrouter', label: t('settings:server.embeddingSourceOpenRouter') },
  ];
  const inAppModels = catalog.filter((m) => m.kind === 'embedding').map((m) => ({ value: m.id, label: m.displayName }));

  return (
    <div className="space-y-4">
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:server.baseUrl')}</label>
        <input
          type="url"
          aria-label={t('settings:server.baseUrl')}
          value={server.baseUrl}
          onChange={(e) => setServer({ baseUrl: e.target.value })}
          placeholder={'http://localhost:1234/v1'} // i18n-ignore: example URL, not user-facing copy
          className={`${INPUT_CLASS} font-mono`}
        />
        <p className="text-xs text-gray-500 mt-1">{t('settings:server.baseUrlHelp')}</p>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">
          {t('settings:server.apiKey')}
          {server.hasApiKey && !clearApiKey && (
            <span className="text-gray-500 font-normal"> {t('settings:ai.apiKeySaved')}</span>
          )}
        </label>
        <input
          type="password"
          aria-label={t('settings:server.apiKey')}
          value={apiKey}
          disabled={clearApiKey}
          onChange={(e) => setApiKey(e.target.value)}
          placeholder={server.hasApiKey ? '••••••••••••••••' : t('settings:server.apiKeyPlaceholder')}
          className={`${INPUT_CLASS} font-mono disabled:opacity-50`}
        />
        {server.hasApiKey && !clearApiKey && (
          <button
            type="button"
            onClick={() => {
              setApiKey('');
              setClearApiKey(true);
            }}
            className="text-xs text-red-600 hover:underline mt-1"
          >
            {t('settings:server.removeKey')}
          </button>
        )}
        {clearApiKey && (
          <p className="text-xs text-amber-600 mt-1">
            {t('settings:server.keyWillBeRemoved')}{' '}
            <button type="button" onClick={() => setClearApiKey(false)} className="underline">
              {t('settings:server.keepKey')}
            </button>
          </p>
        )}
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:ai.chatModel')}</label>
        <div className="flex gap-2">
          <input
            type="text"
            aria-label={t('settings:ai.chatModel')}
            list="server-models"
            value={config.model}
            onChange={(e) => setConfig({ ...config, model: e.target.value })}
            placeholder={t('settings:server.chatModelPlaceholder')}
            className={`${INPUT_CLASS} font-mono`}
          />
          <button
            type="button"
            onClick={onLoadModels}
            disabled={loadingModels || server.baseUrl.trim() === ''}
            className="px-3 py-2 bg-gray-100 text-gray-800 rounded text-sm hover:bg-gray-200 disabled:opacity-50 whitespace-nowrap"
          >
            {loadingModels ? t('common:state.loading') : t('settings:server.loadModels')}
          </button>
        </div>
        <datalist id="server-models">
          {serverModels.map((id) => (
            <option key={id} value={id} />
          ))}
        </datalist>
        <p className="text-xs text-gray-500 mt-1">{t('settings:server.chatModelHelp')}</p>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:server.contextWindow')}</label>
        <input
          type="number"
          aria-label={t('settings:server.contextWindow')}
          min={0}
          step={4096}
          value={server.contextWindow}
          onChange={(e) => {
            const v = parseInt(e.target.value, 10);
            setServer({ contextWindow: Number.isFinite(v) && v > 0 ? v : 0 });
          }}
          className="w-32 bg-white text-gray-800 border border-gray-300 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        />
        <p className="text-xs text-gray-500 mt-1">{t('settings:server.contextWindowHelp')}</p>
        {contextReport &&
          contextReportLines(contextReport).map((line) => (
            <p key={line.key} className={`text-xs mt-1 ${line.warning ? 'text-amber-600' : 'text-gray-600'}`}>
              {t(line.key as 'settings:server.contextLoaded', line.params)}
            </p>
          ))}
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:server.embeddingSource')}</label>
        <Select
          value={server.embeddingSource}
          options={sourceOptions}
          onChange={(source) =>
            setConfig({
              ...config,
              server: { ...server, embeddingSource: source },
              embeddingModel: embeddingModelForSource(source, catalog),
            })
          }
          ariaLabel={t('settings:server.embeddingSource')}
          fullWidth
        />
        <p className="text-xs text-gray-500 mt-1">{t('settings:server.embeddingSourceHelp')}</p>
      </div>
      <div>
        <label className="block text-sm font-medium text-gray-700 mb-1">{t('settings:ai.embeddingModel')}</label>
        {server.embeddingSource === 'server' && (
          <>
            <input
              type="text"
              aria-label={t('settings:ai.embeddingModel')}
              list="server-models"
              value={config.embeddingModel}
              onChange={(e) => setConfig({ ...config, embeddingModel: e.target.value })}
              placeholder={t('settings:server.embeddingModelPlaceholder')}
              className={`${INPUT_CLASS} font-mono`}
            />
            <p className="text-xs text-gray-500 mt-1">{t('settings:server.embeddingModelHelp')}</p>
          </>
        )}
        {server.embeddingSource === 'inapp' && (
          <Select
            value={config.embeddingModel}
            options={inAppModels}
            onChange={(value) => setConfig({ ...config, embeddingModel: value })}
            ariaLabel={t('settings:ai.embeddingModel')}
            fullWidth
          />
        )}
        {server.embeddingSource === 'openrouter' && (
          <>
            <Select
              value={config.embeddingModel}
              options={openRouterEmbeddingSelectOptions(t, openRouterEmbeddingModels, config.embeddingModel)}
              onChange={(value) => setConfig({ ...config, embeddingModel: value })}
              ariaLabel={t('settings:ai.embeddingModel')}
              fullWidth
            />
            <p className="text-xs text-gray-500 mt-1">{t('settings:openRouter.embeddingNotice')}</p>
            {!config.hasApiKey && (
              <p className="text-xs text-gray-500 mt-1">{t('settings:server.openRouterNeedsKey')}</p>
            )}
            {embeddingNeedsCheck && (
              <p className="text-xs text-amber-600 mt-1">{t('settings:openRouter.embeddingNeedsCheck')}</p>
            )}
          </>
        )}
      </div>
      {server.embeddingSource === 'openrouter' && <OpenRouterSpendControls config={config} setConfig={setConfig} />}
      <p className="text-xs text-gray-500">{t('settings:server.privacyNotice')}</p>
      <ThinkingToggle
        enabled={config.thinkingEnabled}
        onToggle={() => setConfig({ ...config, thinkingEnabled: !config.thinkingEnabled })}
      />
    </div>
  );
}
