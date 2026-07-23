// Global meeting-reminder state. The backend fires a `meeting-reminder`
// Tauri event when a meeting is about to start; App.tsx pipes the payload in
// here (the OS notification also fires, but macOS can't deliver its click, so
// the in-app banner is the actionable surface). The banner component reads
// this store and renders on top of every view.

import { create } from 'zustand';
import type { CalendarEvent } from '@/types';

interface ReminderStore {
  reminder: CalendarEvent | null;
  /** Wall-clock ms when the current reminder was shown (drives auto-dismiss). */
  shownAtMs: number | null;
  show: (event: CalendarEvent) => void;
  dismiss: () => void;
}

export const useReminderStore = create<ReminderStore>((set) => ({
  reminder: null,
  shownAtMs: null,
  show: (event) => set({ reminder: event, shownAtMs: Date.now() }),
  dismiss: () => set({ reminder: null, shownAtMs: null }),
}));
