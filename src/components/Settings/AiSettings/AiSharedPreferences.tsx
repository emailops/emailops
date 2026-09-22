// Shared AI preferences that apply across all backends:
// routing mode, keep-alive, AI age cutoff, output language.

import { useTranslation } from 'react-i18next';

import { Select } from '@/components/shared/Select';
import { NATIVE_NAMES, SUPPORTED_LANGUAGES } from '../../../i18n';

import type { RoutingMode } from './types';

interface AiSharedPreferencesProps {
  routingMode: RoutingMode;
  onRoutingModeChange: (mode: RoutingMode) => void;
  keepAliveMinutes: number;
  onKeepAliveChange: (minutes: number) => void;
  aiMaxEmailCount: number;
  onMaxEmailCountChange: (count: number) => void;
  aiMaxEmailAgeDays: number;
  onMaxEmailAgeDaysChange: (days: number) => void;
  /**
   * Context window (tokens) for the embedded llama.cpp chat model. Only shown
   * when {@link showContextWindow} is true — the other backends manage their
   * own context. Stored in the `chat.n_ctx` preference.
   */
  nCtx: number;
  onNCtxChange: (tokens: number) => void;
  showContextWindow: boolean;
  /**
   * Stored value of `ai_output_language_v2`. The empty string is the
   * "Same as UI" sentinel — resolved server-side to the active `ui_language`.
   */
  aiOutputLanguage: string;
  onOutputLanguageChange: (lang: string) => void;
  /** `help_docs_enabled`: let the chat answer questions about EmailOps
   *  itself from the bundled guides (and navigate to the cited section). */
  helpDocsEnabled: boolean;
  onHelpDocsEnabledChange: (enabled: boolean) => void;
}

/**
 * Sentinel for the dropdown's "Same as UI" option. Persisted as the empty
 * string so the backend resolver falls through to `ui_language`. Keep this
 * named (rather than a literal "") so future readers don't think the empty
 * is a bug.
 */
const SAME_AS_UI: '' = '';

export function AiSharedPreferences({
  routingMode,
  onRoutingModeChange,
  keepAliveMinutes,
  onKeepAliveChange,
  aiMaxEmailCount,
  onMaxEmailCountChange,
  aiMaxEmailAgeDays,
  onMaxEmailAgeDaysChange,
  nCtx,
  onNCtxChange,
  showContextWindow,
  aiOutputLanguage,
  onOutputLanguageChange,
  helpDocsEnabled,
  onHelpDocsEnabledChange,
}: AiSharedPreferencesProps) {
  const { t } = useTranslation(['common', 'settings']);

  return (
    <>
      {/* Chat routing mode */}
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.routingMode')}</label>
        <p className="text-xs text-gray-500 mb-2">{t('settings:ai.routingModeHelp')}</p>
        <Select
          value={routingMode}
          options={[
            { value: 'always_rag', label: t('settings:ai.routingAlwaysRag') },
            { value: 'auto', label: t('settings:ai.routingAuto') },
            { value: 'always_tools', label: t('settings:ai.routingAlwaysTools') },
          ]}
          onChange={onRoutingModeChange}
          ariaLabel={t('settings:ai.routingMode')}
          fullWidth
        />
      </div>

      {/* Keep-alive */}
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.keepAlive')}</label>
        <p className="text-xs text-gray-500 mb-2">
          {t('settings:ai.keepAliveHelpStart')} <code>-1</code> {/* i18n-ignore */}{' '}
          {t('settings:ai.keepAliveHelpMiddle')} <code> 0</code> {/* i18n-ignore */} {t('settings:ai.keepAliveHelpEnd')}
        </p>
        <input
          type="number"
          min={-1}
          step={1}
          value={keepAliveMinutes}
          onChange={(e) => {
            const v = parseInt(e.target.value, 10);
            if (Number.isFinite(v)) onKeepAliveChange(v);
          }}
          className="w-32 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        />
      </div>

      {/* Context window — embedded llama.cpp only */}
      {showContextWindow && (
        <div>
          <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.contextWindow')}</label>
          <p className="text-xs text-gray-500 mb-2">{t('settings:ai.contextWindowHelp')}</p>
          <input
            type="number"
            min={1024}
            step={1024}
            value={nCtx}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10);
              if (Number.isFinite(v)) onNCtxChange(v);
            }}
            className="w-32 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
          />
        </div>
      )}

      {/* AI processing limits: whole account up to N emails, else a day cutoff */}
      <div>
        <span className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.ageCutoff')}</span>
        <p className="text-xs text-gray-500 mb-2">{t('settings:ai.ageCutoffHelp')}</p>
        <div className="flex flex-wrap items-end gap-4">
          <label className="block">
            <span className="block text-xs text-gray-400 mb-1">{t('settings:ai.emailCountCutoff')}</span>
            <input
              type="number"
              min={0}
              step={1}
              value={aiMaxEmailCount}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                if (Number.isFinite(v)) onMaxEmailCountChange(Math.max(0, v));
              }}
              className="w-32 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
            />
          </label>
          <label className="block">
            <span className="block text-xs text-gray-400 mb-1">
              {t('settings:ai.ageCutoffDays', { n: aiMaxEmailCount })}
            </span>
            <input
              type="number"
              min={0}
              step={1}
              value={aiMaxEmailAgeDays}
              onChange={(e) => {
                const v = parseInt(e.target.value, 10);
                if (Number.isFinite(v)) onMaxEmailAgeDaysChange(Math.max(0, v));
              }}
              className="w-32 bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
            />
          </label>
        </div>
      </div>

      {/* Output language */}
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.outputLanguage')}</label>
        <p className="text-xs text-gray-500 mb-2">{t('settings:ai.outputLanguageHelp')}</p>
        <Select
          value={aiOutputLanguage}
          options={[
            { value: SAME_AS_UI, label: t('common:language.sameAsUi') },
            ...SUPPORTED_LANGUAGES.map((code) => ({ value: code as string, label: NATIVE_NAMES[code] })),
          ]}
          onChange={onOutputLanguageChange}
          ariaLabel={t('settings:ai.outputLanguage')}
        />
      </div>

      {/* Answer questions about EmailOps from the bundled guides */}
      <div>
        <label className="flex items-start gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={helpDocsEnabled}
            onChange={(e) => onHelpDocsEnabledChange(e.target.checked)}
            className="mt-0.5"
            aria-label={t('settings:ai.helpDocs')}
          />
          <span>
            <span className="block text-sm font-medium text-gray-300">{t('settings:ai.helpDocs')}</span>
            <span className="block text-xs text-gray-500">{t('settings:ai.helpDocsHelp')}</span>
          </span>
        </label>
      </div>
    </>
  );
}
