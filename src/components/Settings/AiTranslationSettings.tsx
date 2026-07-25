import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { errorText } from '@/lib/errors';
import { useTranslationEnabledStore } from '@/stores/featureToggleStore';
import { PromptEditorBlock } from './PromptEditorBlock';

/**
 * Settings panel for AI translation: the feature toggle (backing the same
 * `ai_translation_enabled` preference the Rust commands gate on) plus the
 * user-editable translation prompt.
 */
export function AiTranslationSettings() {
  const { t } = useTranslation(['common', 'settings']);
  const { enabled, isLoading, setEnabled, refresh } = useTranslationEnabledStore();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    refresh().catch((err) => setError(errorText(err)));
  }, [refresh]);

  const handleToggle = () => {
    setError(null);
    setEnabled(!enabled).catch((err) => setError(errorText(err)));
  };

  if (isLoading) {
    return <p className="text-gray-400 text-sm p-4">{t('common:state.loading')}</p>;
  }

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
        {error && <div className="p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">{error}</div>}

        <section>
          <div className="flex items-center justify-between py-2">
            <div>
              <label className="block text-sm font-medium text-gray-300">{t('settings:aiTranslation.enable')}</label>
              <p className="text-xs text-gray-500 mt-0.5">{t('settings:aiTranslation.enableDesc')}</p>
            </div>
            <button
              type="button"
              onClick={handleToggle}
              className={`relative inline-flex h-6 w-11 flex-shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                enabled ? 'bg-primary-600' : 'bg-gray-600'
              }`}
              role="switch"
              aria-checked={enabled}
            >
              <span
                className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                  enabled ? 'translate-x-5' : 'translate-x-0'
                }`}
              />
            </button>
          </div>
        </section>

        {enabled && (
          <section>
            <PromptEditorBlock
              promptId="translate.email"
              title={t('settings:aiTranslation.promptTitle')}
              description={t('settings:aiTranslation.promptDesc')}
            />
          </section>
        )}
      </div>
    </div>
  );
}
