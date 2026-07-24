import { create } from 'zustand';
import { getAvailableUpdate } from '@/lib/api';
import { sanitizeAvailableUpdate, type UpdateAvailablePayload } from '@/lib/appUpdate';

/**
 * Latest-known newer release, backing the persistent download link in the
 * sidebar footer. Populated at startup from the `get_available_update`
 * command (prefs persisted by the daily backend check) and live from the
 * `app-update-available` event. Both paths go through
 * `sanitizeAvailableUpdate`, so `available.url` is always safe to open.
 */
interface UpdateStore {
  available: UpdateAvailablePayload | null;
  setAvailable: (update: UpdateAvailablePayload | null) => void;
  load: () => Promise<void>;
}

export const useUpdateStore = create<UpdateStore>((set) => ({
  available: null,
  setAvailable: (update) => set({ available: update }),
  load: async () => {
    try {
      const update = await getAvailableUpdate();
      set({ available: update ? sanitizeAvailableUpdate(update) : null });
    } catch (err) {
      // Purely informational — a missing update notice is better than an
      // error state (same stance as VersionLabel).
      console.error('Failed to load available update', err);
    }
  },
}));
