import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getBuildInfo } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { formatVersionLabel } from '@/lib/version';
import { useLogStore } from '@/stores/logStore';
import { SettingsPanel } from './SettingsPanel';

const LINKS = [
  { key: 'website', url: 'https://getemailops.com/' },
  { key: 'sourceCode', url: 'https://github.com/emailops/emailops' },
  { key: 'sponsor', url: 'https://github.com/sponsors/emailops' },
] as const;

export function AboutSettings() {
  const { t } = useTranslation(['settings', 'sidebar']);
  const { addLog } = useLogStore();
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getBuildInfo()
      .then((info) => {
        if (!cancelled) setVersion(formatVersionLabel(info));
      })
      .catch((e) => {
        addLog('error', 'system', `Failed to read the build version: ${errorText(e)}`);
      });
    return () => {
      cancelled = true;
    };
  }, [addLog]);

  const openLink = (url: string) => {
    void openExternal(url).catch((e) => {
      addLog('error', 'system', `Failed to open ${url}: ${errorText(e)}`);
    });
  };

  return (
    <SettingsPanel>
      <section>
        <h3 className="text-base font-semibold text-gray-900">{t('sidebar:appName')}</h3>
        {version && <p className="text-xs text-gray-500 mt-0.5">{version}</p>}
        <p className="text-sm text-gray-700 mt-3">{t('settings:about.tagline')}</p>
      </section>
      <section className="flex flex-col items-start gap-2">
        {LINKS.map((link) => (
          <button
            key={link.key}
            type="button"
            onClick={() => openLink(link.url)}
            className="text-sm text-primary-600 hover:text-primary-700 hover:underline"
          >
            {t(`settings:about.${link.key}` as const)}
          </button>
        ))}
      </section>
    </SettingsPanel>
  );
}
