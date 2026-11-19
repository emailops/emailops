import { DataLocationSection } from '@/components/DataLocation/DataLocationSection';
import { BackupsSection } from './BackupsSection';
import { SettingsPanel } from './SettingsPanel';

/** Settings → Data & Backups: where the data folder is, and its backups. */
export function DataSettings() {
  return (
    <SettingsPanel>
      <DataLocationSection />
      <BackupsSection />
    </SettingsPanel>
  );
}
