import { useTranslation } from 'react-i18next';
import { SettingsPanel } from './SettingsPanel';

interface LensesSettingsProps {
  /** Master switch for AI Lenses. Mirrors the `lenses_enabled` SQLite
   *  preference — toggling it both hides the sidebar entry and gates the
   *  Lenses view. */
  enabled: boolean;
  onChangeEnabled: (enabled: boolean) => void;
}

export function LensesSettings({ enabled, onChangeEnabled }: LensesSettingsProps) {
  const { t } = useTranslation(['common', 'settings']);

  return (
    <SettingsPanel>
      {/* Header — owns the master enable toggle. */}
      <section className="p-3 rounded-lg border border-gray-700 bg-[#1f1f20]">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium text-gray-100">{t('settings:lenses.title')}</span>
            </div>
            <p className="text-xs text-gray-400 mt-1">{t('settings:lenses.experimentalDesc')}</p>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            onClick={() => onChangeEnabled(!enabled)}
            className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors mt-0.5 ${
              enabled ? 'bg-primary-600' : 'bg-neutral-600'
            }`}
          >
            <span
              className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                enabled ? 'translate-x-5' : 'translate-x-1'
              }`}
            />
          </button>
        </div>
      </section>

      {enabled ? (
        <section className="rounded-lg border border-gray-700 bg-[#1f1f20] px-4 py-3">
          <p className="text-xs text-gray-400">{t('settings:lenses.enabledInfo')}</p>
        </section>
      ) : (
        <p className="text-xs text-gray-500 italic">{t('settings:lenses.enablePrompt')}</p>
      )}
    </SettingsPanel>
  );
}
