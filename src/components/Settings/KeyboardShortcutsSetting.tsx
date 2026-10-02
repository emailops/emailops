import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ToggleSwitch } from '@/components/common/ToggleSwitch';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { useShortcutStore } from '@/stores/shortcutStore';

/** Keyboard shortcuts on/off (default on), stored in the SQLite preferences,
 *  with a way to open the `?` list from here. */
export function KeyboardShortcutsSetting() {
  const { t } = useTranslation(['settings']);
  const enabled = useShortcutStore((s) => s.enabled);
  const setEnabled = useShortcutStore((s) => s.setEnabled);
  const setHelpOpen = useShortcutStore((s) => s.setHelpOpen);
  const addLog = useLogStore((s) => s.addLog);
  const [error, setError] = useState<string | null>(null);

  const change = async (next: boolean) => {
    setError(null);
    try {
      await setEnabled(next);
      addLog('success', 'system', `Keyboard shortcuts ${next ? 'on' : 'off'}`);
    } catch (err) {
      setError(errorText(err));
      addLog('error', 'system', `Could not save the keyboard-shortcuts setting: ${errorText(err)}`);
    }
  };

  return (
    <section data-testid="keyboard-shortcuts-setting">
      <ToggleSwitch
        checked={enabled}
        onChange={(next) => void change(next)}
        label={t('settings:appearance.keyboardShortcuts')}
        description={t('settings:appearance.keyboardShortcutsHelp')}
        ariaLabel={t('settings:appearance.keyboardShortcuts')}
      />
      <button
        type="button"
        data-testid="keyboard-shortcuts-show"
        onClick={() => setHelpOpen(true)}
        className="mt-2 text-xs text-primary-400 hover:text-primary-300 hover:underline"
      >
        {t('settings:appearance.keyboardShortcutsShowList')}
      </button>
      {error && <p className="mt-2 text-xs text-red-400">{error}</p>}
    </section>
  );
}
