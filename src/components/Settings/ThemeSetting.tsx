import { useTranslation } from 'react-i18next';
import { ThemeChoice } from '@/components/shared/ThemeChoice';

/**
 * Settings → Appearance → Theme: the shared three-way choice (light, dark,
 * follow the system) under its heading. The onboarding's view step shows the
 * same `ThemeChoice`.
 */
export function ThemeSetting() {
  const { t } = useTranslation(['settings']);
  return (
    <section>
      <h3 className="text-sm font-semibold text-gray-300 mb-3">{t('settings:appearance.theme')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.themeHelp')}</p>
      <ThemeChoice ariaLabel={t('settings:appearance.theme')} />
    </section>
  );
}
