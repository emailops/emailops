import { useTranslation } from 'react-i18next';

interface LensesSettingsProps {
  /** Master switch for the experimental AI Lenses feature. Mirrors the
   *  `lenses_enabled` SQLite preference — toggling it both hides the sidebar
   *  entry and gates the Lenses view. */
  experimentalEnabled: boolean;
  onChangeExperimentalEnabled: (enabled: boolean) => void;
}

export function LensesSettings({ experimentalEnabled, onChangeExperimentalEnabled }: LensesSettingsProps) {
  const { t } = useTranslation(['common', 'settings']);

  return (
    <div className="flex flex-col flex-1 min-h-0">
      <div className="overflow-y-auto flex-1 px-6 py-5 space-y-6">
        {/* Experimental header — owns the master enable toggle. */}
        <section className="p-3 rounded-lg border border-amber-700/50 bg-amber-900/10">
          <div className="flex items-start justify-between gap-4">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium text-gray-100">{t('settings:lenses.title')}</span>
                <span className="px-1.5 py-0.5 rounded text-[10px] font-semibold uppercase tracking-wider bg-amber-900/40 text-amber-300 border border-amber-700/50">
                  {t('settings:dialog.experimental')}
                </span>
              </div>
              <p className="text-xs text-gray-400 mt-1">{t('settings:lenses.experimentalDesc')}</p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={experimentalEnabled}
              onClick={() => onChangeExperimentalEnabled(!experimentalEnabled)}
              className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors mt-0.5 ${
                experimentalEnabled ? 'bg-primary-600' : 'bg-neutral-600'
              }`}
            >
              <span
                className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                  experimentalEnabled ? 'translate-x-5' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        </section>

        {experimentalEnabled ? (
          <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
            <p className="text-xs text-gray-400">{t('settings:lenses.enabledInfo')}</p>
          </section>
        ) : (
          <p className="text-xs text-gray-500 italic">{t('settings:lenses.enablePrompt')}</p>
        )}
      </div>
    </div>
  );
}
