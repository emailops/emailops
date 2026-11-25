import { create } from 'zustand';

/** A calendar changed outside the calendar views (an invite answered from an
 *  email). `seq` grows with every change, so the same account changing twice
 *  is still a new change. */
export interface CalendarChange {
  accountId: string;
  seq: number;
}

interface CalendarChangeStore {
  lastChange: CalendarChange | null;
  markChanged: (accountId: string) => void;
}

export const useCalendarChangeStore = create<CalendarChangeStore>((set) => ({
  lastChange: null,
  markChanged: (accountId) => set((s) => ({ lastChange: { accountId, seq: (s.lastChange?.seq ?? 0) + 1 } })),
}));
