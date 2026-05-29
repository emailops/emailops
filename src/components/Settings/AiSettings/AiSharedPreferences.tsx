// Shared AI preferences that apply across all backends:
// routing mode, keep-alive, AI age cutoff, output language.

import { useTranslation } from 'react-i18next';

import { NATIVE_NAMES, SUPPORTED_LANGUAGES } from '../../../i18n';

import type { RoutingMode } from './types';

interface AiSharedPreferencesProps {
  routingMode: RoutingMode;
  onRoutingModeChange: (mode: RoutingMode) => void;
  keepAliveMinutes: number;
  onKeepAliveChange: (minutes: number) => void;
  aiMaxEmailAgeDays: number;
  onMaxEmailAgeDaysChange: (days: number) => void;
  /**
   * Stored value of `ai_output_language_v2`. The empty string is the
   * "Same as UI" sentinel — resolved server-side to the active `ui_language`.
   */
  aiOutputLanguage: string;
  onOutputLanguageChange: (lang: string) => void;
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
  aiMaxEmailAgeDays,
  onMaxEmailAgeDaysChange,
  aiOutputLanguage,
  onOutputLanguageChange,
}: AiSharedPreferencesProps) {
  const { t } = useTranslation(['common', 'settings']);

  return (
    <>
      {/* Chat routing mode */}
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.routingMode')}</label>
        <p className="text-xs text-gray-500 mb-2">{t('settings:ai.routingModeHelp')}</p>
        <select
          value={routingMode}
          onChange={(e) => onRoutingModeChange(e.target.value as RoutingMode)}
          className="w-full bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        >
          <option value="always_rag">{t('settings:ai.routingAlwaysRag')}</option>
          <option value="auto">{t('settings:ai.routingAuto')}</option>
          <option value="always_tools">{t('settings:ai.routingAlwaysTools')}</option>
        </select>
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

      {/* AI processing age cutoff */}
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.ageCutoff')}</label>
        <p className="text-xs text-gray-500 mb-2">{t('settings:ai.ageCutoffHelp')}</p>
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
      </div>

      {/* Output language */}
      <div>
        <label className="block text-sm font-medium text-gray-300 mb-1">{t('settings:ai.outputLanguage')}</label>
        <p className="text-xs text-gray-500 mb-2">{t('settings:ai.outputLanguageHelp')}</p>
        <select
          value={aiOutputLanguage}
          onChange={(e) => onOutputLanguageChange(e.target.value)}
          className="bg-[#333] text-gray-200 border border-gray-600 rounded px-3 py-2 text-sm focus:border-primary-500 outline-none"
        >
          <option value={SAME_AS_UI}>{t('common:language.sameAsUi')}</option>
          {SUPPORTED_LANGUAGES.map((code) => (
            <option key={code} value={code}>
              {NATIVE_NAMES[code]}
            </option>
          ))}
        </select>
      </div>
    </>
  );
}
