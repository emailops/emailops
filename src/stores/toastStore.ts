import { create } from 'zustand';

export interface Toast {
  id: number;
  message: string;
  /** Optional action button label (e.g. "Show in Finder"). */
  actionLabel?: string;
  onAction?: () => void;
  /** Sticky toasts never auto-dismiss — only the user can close them
   *  (e.g. the app-update notification). */
  sticky?: boolean;
}

interface ToastStore {
  toasts: Toast[];
  nextId: number;
  addToast: (toast: Omit<Toast, 'id'>) => number;
  dismissToast: (id: number) => void;
}

const MAX_TOASTS = 5;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],
  nextId: 1,

  addToast: (toast) => {
    const id = get().nextId;
    set((state) => {
      const toasts = [...state.toasts, { ...toast, id }];
      if (toasts.length > MAX_TOASTS) {
        toasts.splice(0, toasts.length - MAX_TOASTS);
      }
      return { toasts, nextId: state.nextId + 1 };
    });
    return id;
  },

  dismissToast: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
}));
