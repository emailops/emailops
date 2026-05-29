import { useTranslation } from 'react-i18next';

interface ThinkingToggleProps {
  enabled: boolean;
  onToggle: () => void;
}

export function ThinkingToggle({ enabled, onToggle }: ThinkingToggleProps) {
  const { t } = useTranslation(['common', 'settings']);
  return (
    <div className="flex items-center justify-between">
      <div>
        <label className="block text-sm font-medium text-gray-300">{t('settings:ai.thinkingMode')}</label>
        <p className="text-xs text-gray-500 mt-0.5">{t('settings:ai.thinkingModeHelp')}</p>
      </div>
      <button
        type="button"
        onClick={onToggle}
        className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
          enabled ? 'bg-primary-600' : 'bg-gray-600'
        }`}
      >
        <span
          className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
            enabled ? 'translate-x-5' : 'translate-x-0'
          }`}
        />
      </button>
    </div>
  );
}
