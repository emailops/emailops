import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { formatBytes } from '@/lib/intl';
import { useLogStore } from '@/stores/logStore';
import { DataMoveDialog } from './DataMoveDialog';
import { useDataMove } from './useDataMove';

/** Settings → Privacy: where the data folder is, and moving it. */
export function DataLocationSection() {
  const { t, i18n } = useTranslation(['settings']);
  const { addLog } = useLogStore();
  const [info, setInfo] = useState<api.DataLocationInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { state, pickError, pick, restoreDefault, confirm, cancel } = useDataMove();

  useEffect(() => {
    api
      .getDataLocation()
      .then(setInfo)
      .catch((e) => {
        setError(errorText(e));
        addLog('error', 'system', `Failed to read the data location: ${errorText(e)}`);
      });
  }, [addLog]);

  const dismissPrevious = useCallback(async () => {
    try {
      await api.dismissPreviousDataLocation();
      setInfo((current) => (current ? { ...current, previousPath: null } : current));
    } catch (e) {
      setError(errorText(e));
      addLog('error', 'system', `Failed to dismiss the old data folder note: ${errorText(e)}`);
    }
  }, [addLog]);

  if (info && !info.supported) return null;
  const shownError = error ?? pickError;

  return (
    <section>
      <h3 className="text-sm font-semibold text-gray-700 mb-1">{t('settings:privacy.dataLocation.title')}</h3>
      <p className="text-xs text-gray-500 mb-3">{t('settings:privacy.dataLocation.help')}</p>
      {shownError && (
        <div className="mb-3 p-3 bg-red-50 border border-red-200 rounded text-red-700 text-sm" role="alert">
          {shownError}
        </div>
      )}
      {info && (
        <div className="rounded-lg border border-gray-200 bg-white px-4 py-3 space-y-3">
          <div>
            <div className="break-all font-mono text-xs text-gray-800">{info.path}</div>
            <div className="text-xs text-gray-500 mt-0.5">
              {t('settings:privacy.dataLocation.size', { size: formatBytes(info.sizeBytes, i18n.language) })}
            </div>
          </div>
          {info.setByEnv && <p className="text-xs text-gray-500">{t('settings:privacy.dataLocation.setByEnv')}</p>}
          <div className="flex flex-wrap gap-3">
            <button
              type="button"
              data-testid="data-location-change"
              onClick={() => void pick()}
              disabled={info.setByEnv}
              className="text-sm text-primary-600 hover:text-primary-700 transition-colors disabled:opacity-50"
            >
              {t('settings:privacy.dataLocation.change')}
            </button>
            {!info.isDefault && !info.setByEnv && (
              <button
                type="button"
                data-testid="data-location-restore"
                onClick={() => restoreDefault(info)}
                className="text-sm text-primary-600 hover:text-primary-700 transition-colors"
              >
                {t('settings:privacy.dataLocation.restoreDefault')}
              </button>
            )}
          </div>
          {info.previousPath && (
            <div className="rounded bg-gray-100 px-3 py-2 text-xs text-gray-700">
              <p>{t('settings:privacy.dataLocation.previousCopy')}</p>
              <p className="mt-1 break-all font-mono">{info.previousPath}</p>
              <button
                type="button"
                data-testid="data-location-dismiss-previous"
                onClick={() => void dismissPrevious()}
                className="mt-1 text-primary-600 hover:text-primary-700 transition-colors"
              >
                {t('settings:privacy.dataLocation.dismissPrevious')}
              </button>
            </div>
          )}
        </div>
      )}
      <DataMoveDialog state={state} onConfirm={() => void confirm()} onCancel={cancel} />
    </section>
  );
}
