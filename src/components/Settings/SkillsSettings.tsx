import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listSkills, openSkillsFolder, type SkillsOverview, setBundledSkillEnabled } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useSkillsEnabledStore } from '@/stores/featureToggleStore';
import { useLogStore } from '@/stores/logStore';
import { SkillSwitch } from '../Skills/SkillSwitch';
import { SettingsPanel } from './SettingsPanel';

/**
 * Settings panel for chat skills: the feature toggle (the `skills_enabled`
 * preference the backend gates on), the skills folder, and what was found in
 * it. Skills are edited on disk — each is a folder holding a `SKILL.md` — so
 * this panel only lists them, opens the folder and re-reads it. It also lists
 * the skills that ship with the app, even with the feature off: switching one
 * on then offers to turn the feature on, and changes nothing until accepted.
 */
export function SkillsSettings() {
  const { t } = useTranslation(['common', 'settings']);
  const addLog = useLogStore((s) => s.addLog);
  const { enabled, isLoading, setEnabled, refresh } = useSkillsEnabledStore();
  const [overview, setOverview] = useState<SkillsOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The bundled skill waiting for the user to accept turning the feature on.
  const [offered, setOffered] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setError(null);
    try {
      setOverview(await listSkills());
    } catch (err) {
      const msg = errorText(err);
      setError(msg);
      addLog('error', 'ai', `Failed to list skills: ${msg}`);
    }
  }, [addLog]);

  useEffect(() => {
    refresh().catch((err) => setError(errorText(err)));
    void reload();
  }, [refresh, reload]);

  const handleToggle = () => {
    setError(null);
    setEnabled(!enabled).catch((err) => setError(errorText(err)));
  };

  const switchBundled = async (name: string, on: boolean) => {
    setError(null);
    try {
      await setBundledSkillEnabled(name, on);
      addLog('info', 'ai', `Bundled skill ${name} ${on ? 'enabled' : 'disabled'}`);
      await reload();
    } catch (err) {
      const msg = errorText(err);
      setError(msg);
      addLog('error', 'ai', `Failed to switch bundled skill ${name}: ${msg}`);
    }
  };

  const handleBundledToggle = (name: string, on: boolean) => {
    if (on && !enabled) {
      setOffered(name);
      return;
    }
    void switchBundled(name, on);
  };

  const handleAcceptOffer = async () => {
    if (!offered) return;
    const name = offered;
    setOffered(null);
    setError(null);
    try {
      await setEnabled(true);
    } catch (err) {
      setError(errorText(err));
      return;
    }
    await switchBundled(name, true);
  };

  const handleOpenFolder = async () => {
    try {
      await openSkillsFolder();
      addLog('info', 'ai', 'Opened the skills folder');
      // Opening creates the folder when it was missing — show its path.
      await reload();
    } catch (err) {
      const msg = errorText(err);
      setError(msg);
      addLog('error', 'ai', `Failed to open the skills folder: ${msg}`);
    }
  };

  if (isLoading) {
    return <p className="text-gray-500 text-sm p-4">{t('common:state.loading')}</p>;
  }

  return (
    <SettingsPanel>
      {error && <div className="p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm">{error}</div>}

      <section>
        <div className="flex items-center justify-between py-2">
          <div>
            <label className="block text-sm font-medium text-gray-700">{t('settings:skills.enable')}</label>
            <p className="text-xs text-gray-500 mt-0.5">{t('settings:skills.enableDesc')}</p>
          </div>
          <button
            type="button"
            onClick={handleToggle}
            className={`relative inline-flex h-6 w-11 flex-shrink-0 rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
              enabled ? 'bg-primary-600' : 'bg-gray-300'
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

      <section className="space-y-2">
        <h3 className="text-sm font-medium text-gray-700">{t('settings:skills.folderTitle')}</h3>
        <p className="text-xs text-gray-500">{t('settings:skills.folderDesc')}</p>
        {overview?.dir && <code className="block text-xs text-gray-500 break-all">{overview.dir}</code>}
        <div className="flex gap-2">
          <button
            type="button"
            data-testid="skills-open-folder"
            onClick={() => void handleOpenFolder()}
            className="px-3 py-1.5 text-xs rounded bg-gray-100 hover:bg-gray-200 text-gray-800"
          >
            {t('settings:skills.openFolder')}
          </button>
          <button
            type="button"
            data-testid="skills-reload"
            onClick={() => void reload()}
            className="px-3 py-1.5 text-xs rounded bg-gray-100 hover:bg-gray-200 text-gray-800"
          >
            {t('settings:skills.reload')}
          </button>
        </div>
      </section>

      <section className="space-y-2">
        <h3 className="text-sm font-medium text-gray-700">{t('settings:skills.listTitle')}</h3>
        {overview && overview.skills.length === 0 && overview.errors.length === 0 && (
          <p className="text-xs text-gray-500">{t('settings:skills.empty')}</p>
        )}
        <ul className="space-y-2">
          {overview?.skills.map((s) => (
            <li key={s.name} className="text-sm">
              <code className="text-primary-600">/{s.name}</code>
              <p className="text-xs text-gray-500 mt-0.5">{s.description}</p>
            </li>
          ))}
        </ul>
        {overview && overview.errors.length > 0 && (
          <div className="space-y-1">
            <p className="text-xs font-medium text-red-700">{t('settings:skills.errorsTitle')}</p>
            <ul className="space-y-1">
              {overview.errors.map((e) => (
                <li key={e.path} className="text-xs text-red-700">
                  <code className="break-all">{e.path}</code>: {errorText(e)}
                </li>
              ))}
            </ul>
          </div>
        )}
      </section>

      {overview && overview.bundled.length > 0 && (
        <section className="space-y-2">
          <h3 className="text-sm font-medium text-gray-700">{t('settings:skills.bundledTitle')}</h3>
          <p className="text-xs text-gray-500">{t('settings:skills.bundledDesc')}</p>
          {offered && (
            <div
              data-testid="skills-offer"
              className="flex items-center gap-2 p-3 bg-amber-50 border border-amber-200 rounded text-sm text-amber-800"
            >
              <span className="flex-1">{t('settings:skills.offer', { name: offered })}</span>
              <button
                type="button"
                data-testid="skills-offer-yes"
                onClick={() => void handleAcceptOffer()}
                className="px-3 py-1.5 text-xs rounded bg-primary-600 hover:bg-primary-500 text-white"
              >
                {t('settings:skills.offerYes')}
              </button>
              <button
                type="button"
                data-testid="skills-offer-no"
                onClick={() => setOffered(null)}
                className="px-3 py-1.5 text-xs rounded bg-gray-100 hover:bg-gray-200 text-gray-800"
              >
                {t('settings:skills.offerNo')}
              </button>
            </div>
          )}
          <ul className="space-y-2">
            {overview.bundled.map((b) => (
              <li key={b.name} className="flex items-start gap-3 text-sm">
                <div className="flex-1 min-w-0">
                  <code className="text-primary-600">/{b.name}</code>
                  <p className="text-xs text-gray-500 mt-0.5">
                    {b.shadowed ? t('settings:skills.view.bundledShadowed') : b.description}
                  </p>
                </div>
                {!b.shadowed && (
                  <SkillSwitch
                    checked={b.enabled}
                    label={t('settings:skills.view.toggle', { name: b.name })}
                    testId={`bundled-toggle-${b.name}`}
                    onToggle={() => handleBundledToggle(b.name, !b.enabled)}
                  />
                )}
              </li>
            ))}
          </ul>
        </section>
      )}
    </SettingsPanel>
  );
}
