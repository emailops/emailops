import { listen } from '@tauri-apps/api/event';
import { useCallback, useEffect, useReducer, useState } from 'react';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { dataMoveReducer, IDLE } from './dataMoveFlow';

/** The pick → confirm → move flow shared by Settings and onboarding. */
export function useDataMove() {
  const [state, dispatch] = useReducer(dataMoveReducer, IDLE);
  const [pickError, setPickError] = useState<string | null>(null);
  const { addLog } = useLogStore();

  useEffect(() => {
    if (state.phase !== 'moving') return;
    const unlisten = listen<api.DataMoveProgress>('data-move-progress', (event) => {
      const { copiedBytes, totalBytes } = event.payload ?? {};
      if (typeof copiedBytes !== 'number' || typeof totalBytes !== 'number') return;
      dispatch({ type: 'progress', copiedBytes, totalBytes });
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, [state.phase]);

  const pick = useCallback(async () => {
    setPickError(null);
    try {
      const picked = await api.pickDataFolder();
      if (picked) dispatch({ type: 'picked', toDefault: false, target: picked.path, neededBytes: picked.neededBytes });
    } catch (e) {
      setPickError(errorText(e));
      addLog('error', 'system', `Cannot move the data there: ${errorText(e)}`);
    }
  }, [addLog]);

  const restoreDefault = useCallback((info: api.DataLocationInfo) => {
    setPickError(null);
    dispatch({ type: 'picked', toDefault: true, target: info.defaultPath, neededBytes: info.sizeBytes });
  }, []);

  const confirm = useCallback(async () => {
    if (state.phase !== 'confirm') return;
    dispatch({ type: 'confirmed' });
    addLog('info', 'system', 'Moving the data folder');
    try {
      // Restarts the app on success; it only returns by failing.
      await api.moveDataLocation(state.toDefault);
    } catch (e) {
      dispatch({ type: 'failed', error: errorText(e) });
      addLog('error', 'system', `Moving the data folder failed: ${errorText(e)}`);
    }
  }, [state, addLog]);

  const cancel = useCallback(() => dispatch({ type: 'cancelled' }), []);

  return { state, pickError, pick, restoreDefault, confirm, cancel };
}
