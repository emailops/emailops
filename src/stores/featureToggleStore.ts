import { create } from 'zustand';
import * as api from '@/lib/api';

/**
 * Feature toggles for the experimental Memory and Tasks pipelines.
 *
 * These mirror the `memory_enabled` and `task_enabled` rows in the SQLite
 * `user_preferences` table — the same keys the backend extractor reads to
 * decide whether to run the LLM pipeline. Unifying the sidebar visibility
 * flag and the extraction gate behind ONE preference key fixes the long-
 * standing bug where toggling the "experimental" switch in Settings only
 * hid the sidebar entry while the backend pipeline kept extracting.
 *
 * Per CLAUDE.md, components must destructure reactive fields
 * (`const { enabled } = useMemoryEnabledStore()`) so React subscribes to
 * changes; reading via `getState().enabled` inside `useMemo`/`useEffect`
 * deps will silently miss updates.
 */
interface BoolPrefStore {
  enabled: boolean;
  /** True until the first `refresh()` resolves — lets the app defer rendering
   *  conditional UI until the persisted value is known. */
  isLoading: boolean;
  refresh: () => Promise<void>;
  setEnabled: (value: boolean) => Promise<void>;
}

function createBoolPrefStore(key: string, defaultValue: boolean) {
  return create<BoolPrefStore>((set) => ({
    enabled: defaultValue,
    isLoading: true,
    refresh: async () => {
      try {
        const raw = await api.getPref(key);
        const enabled = raw === null ? defaultValue : raw.toLowerCase() === 'true';
        set({ enabled, isLoading: false });
      } catch (err) {
        console.error(`Failed to load pref "${key}":`, err);
        set({ isLoading: false });
      }
    },
    setEnabled: async (value) => {
      // Optimistically reflect in the UI; revert on persistence failure so the
      // toggle doesn't lie about its state.
      set({ enabled: value });
      try {
        await api.setPref(key, value ? 'true' : 'false');
      } catch (err) {
        set({ enabled: !value });
        throw err;
      }
    },
  }));
}

export const useMemoryEnabledStore = createBoolPrefStore('memory_enabled', false);
export const useTasksEnabledStore = createBoolPrefStore('task_enabled', false);
export const useLensesEnabledStore = createBoolPrefStore('lenses_enabled', false);
