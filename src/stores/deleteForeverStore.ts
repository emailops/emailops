import { create } from 'zustand';
import type { DeleteForeverRequest } from '@/components/Inbox/DeleteForeverDialog';

/**
 * The "Delete forever" / "Empty" request whose dialog is open. Kept outside
 * the bulk toolbar, which unmounts as soon as the selection clears, so the
 * dialog stays up for the whole run and its summary.
 */
interface DeleteForeverStore {
  request: DeleteForeverRequest | null;
  open: (request: DeleteForeverRequest) => void;
  close: () => void;
}

export const useDeleteForeverStore = create<DeleteForeverStore>((set) => ({
  request: null,
  open: (request) => set({ request }),
  close: () => set({ request: null }),
}));
