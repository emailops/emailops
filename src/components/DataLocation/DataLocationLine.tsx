import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { DataMoveDialog } from './DataMoveDialog';
import { useDataMove } from './useDataMove';

/**
 * Onboarding step 1: one discreet line naming the data folder, with a link to
 * move it. Most users never need it, so it is not a wizard screen of its own.
 * It sits before any model download, so moving here copies almost nothing;
 * the app restarts and the wizard reopens on this step showing the new folder.
 */
export function DataLocationLine() {
  const { t } = useTranslation(['settings']);
  const { addLog } = useLogStore();
  const [info, setInfo] = useState<api.DataLocationInfo | null>(null);
  const { state, pickError, pick, confirm, cancel } = useDataMove();

  useEffect(() => {
    api
      .getDataLocation()
      .then(setInfo)
      .catch((e) => {
        // The line is optional; onboarding goes on without it.
        addLog('error', 'system', `Failed to read the data location: ${errorText(e)}`);
      });
  }, [addLog]);

  if (!info?.supported) return null;

  return (
    <div className="text-xs text-gray-500">
      <span>{t('settings:privacy.dataLocation.onboardingLine')} </span>
      <span className="break-all font-mono text-gray-700">{info.path}</span>
      {!info.setByEnv && (
        <>
          {' · '}
          <button
            type="button"
            data-testid="data-location-change"
            onClick={() => void pick()}
            className="text-primary-600 hover:text-primary-700 transition-colors"
          >
            {t('settings:privacy.dataLocation.change')}
          </button>
        </>
      )}
      {pickError && (
        <p className="mt-1 text-red-600" role="alert">
          {pickError}
        </p>
      )}
      <DataMoveDialog state={state} onConfirm={() => void confirm()} onCancel={cancel} />
    </div>
  );
}
