import { create } from 'zustand';
import * as api from '@/lib/api';

/**
 * Master AI enable/disable flag, mirrored from the `ai_enabled` row in the
 * SQLite `user_preferences` table. The store starts with `enabled: true` so
 * UI rendered before `refresh()` resolves does not flicker AI surfaces off
 * on every cold start (the backend defaults to enabled for the same reason).
 *
 * IMPORTANT: per CLAUDE.md, components must destructure `enabled` from this
 * hook (`const { enabled } = useAiStore()`) so React subscribes to changes.
 * Reading via `useAiStore.getState().enabled` inside `useMemo`/`useEffect`
 * dependencies will silently miss updates.
 */
interface AiStore {
  enabled: boolean;
  /** True until the first `refresh()` resolves — lets the app defer rendering
   *  AI-conditional UI until we know the persisted value. */
  isLoading: boolean;
  /** Reload the flag from the backend. Call once on app boot. */
  refresh: () => Promise<void>;
  /** Persist the flag and update local state in one step. */
  setEnabled: (value: boolean) => Promise<void>;
}

export const useAiStore = create<AiStore>((set) => ({
  enabled: true,
  isLoading: true,
  refresh: async () => {
    const raw = await api.getPref('ai_enabled');
    // Missing row → enabled (preserves prior behaviour for upgrading users).
    const enabled = raw === null ? true : raw.toLowerCase() === 'true';
    set({ enabled, isLoading: false });
  },
  setEnabled: async (value) => {
    await api.setPref('ai_enabled', value ? 'true' : 'false');
    set({ enabled: value });
  },
}));
