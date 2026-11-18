import { useTranslation } from 'react-i18next';
import { Select } from '@/components/shared/Select';
import type { SidebarDensity } from '@/lib/sidebarDensity';
import { useLogStore } from '@/stores/logStore';

/**
 * "Sidebar density": Comfortable (default) or Compact, which tightens the
 * vertical spacing of the sidebar's scrolling list. The value is owned by App
 * (a `sidebar_density` SQLite preference) and passed in, so the sidebar
 * changes as soon as the user picks another one. Saving happens in App's
 * `usePersistedPref`, which logs a failed write itself.
 */
export function SidebarDensitySetting({
  value,
  onChange,
}: {
  value: SidebarDensity;
  onChange: (density: SidebarDensity) => void;
}) {
  const { t } = useTranslation(['settings']);
  const addLog = useLogStore((s) => s.addLog);

  const options: { value: SidebarDensity; label: string }[] = [
    { value: 'comfortable', label: t('settings:appearance.sidebarDensityComfortable') },
    { value: 'compact', label: t('settings:appearance.sidebarDensityCompact') },
  ];

  const change = (next: SidebarDensity) => {
    onChange(next);
    addLog('info', 'system', `Sidebar density: ${next}`);
  };

  return (
    <section data-testid="sidebar-density-setting">
      <h3 className="text-sm font-semibold text-gray-700 mb-3">{t('settings:appearance.sidebarDensity')}</h3>
      <p className="text-xs text-gray-500 mb-2">{t('settings:appearance.sidebarDensityHelp')}</p>
      <Select
        ariaLabel={t('settings:appearance.sidebarDensity')}
        testId="sidebar-density-select"
        value={value}
        options={options}
        onChange={change}
      />
    </section>
  );
}
