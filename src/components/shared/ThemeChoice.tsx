import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { errorText } from '@/lib/errors';
import { THEME_PREFERENCES, type ThemePreference } from '@/lib/theme';
import { useLogStore } from '@/stores/logStore';
import { useThemeStore } from '@/stores/themeStore';

/**
 * The three theme cards — light, dark, or follow the system — wired to
 * `useThemeStore`. Shared by Settings → Appearance and the onboarding's view
 * step, so both show the same choice and persist it the same way: the click
 * applies at once (the store is optimistic) and writes the `ui_theme`
 * preference; a failed write puts the previous choice back and lands in the
 * output panel.
 */
export function ThemeChoice({ ariaLabel }: { ariaLabel: string }) {
  const { t } = useTranslation(['settings']);
  const { preference, setPreference } = useThemeStore();
  const { addLog } = useLogStore();

  const choose = (next: ThemePreference) => {
    setPreference(next).catch((e) => {
      addLog('error', 'system', `Could not save the theme setting: ${errorText(e)}`);
    });
  };

  return (
    <div className="grid grid-cols-3 gap-3" role="radiogroup" aria-label={ariaLabel}>
      {THEME_PREFERENCES.map((value) => (
        <ThemeOption
          key={value}
          selected={preference === value}
          onClick={() => choose(value)}
          label={t(LABEL_KEY[value])}
          description={t(DESC_KEY[value])}
          icon={ICONS[value]}
        />
      ))}
    </div>
  );
}

const LABEL_KEY = {
  light: 'settings:appearance.themeLight',
  dark: 'settings:appearance.themeDark',
  system: 'settings:appearance.themeSystem',
} as const;

const DESC_KEY = {
  light: 'settings:appearance.themeLightDesc',
  dark: 'settings:appearance.themeDarkDesc',
  system: 'settings:appearance.themeSystemDesc',
} as const;

// Module-scoped on purpose: a component defined inside another gets a new
// identity every render and remounts (src/CLAUDE.md, "Component Identity").
function ThemeOption({
  selected,
  onClick,
  label,
  description,
  icon,
}: {
  selected: boolean;
  onClick: () => void;
  label: string;
  description: string;
  icon: ReactNode;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onClick}
      className={`flex flex-col items-start gap-2 p-3 rounded-lg border-2 text-left transition-colors ${
        selected ? 'border-primary-500 bg-primary-50' : 'border-gray-200 bg-white hover:border-gray-300'
      }`}
    >
      <span className={selected ? 'text-primary-600' : 'text-gray-500'}>{icon}</span>
      <span className={`text-sm font-medium ${selected ? 'text-gray-900' : 'text-gray-700'}`}>{label}</span>
      <span className="text-xs text-gray-500">{description}</span>
    </button>
  );
}

const SunIcon = (
  <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
  </svg>
);

const MoonIcon = (
  <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
    <path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z" />
  </svg>
);

const MonitorIcon = (
  <svg viewBox="0 0 24 24" className="w-5 h-5" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
    <rect x="3" y="4" width="18" height="12" rx="2" />
    <path d="M8 20h8M12 16v4" />
  </svg>
);

const ICONS: Record<ThemePreference, ReactNode> = { light: SunIcon, dark: MoonIcon, system: MonitorIcon };
