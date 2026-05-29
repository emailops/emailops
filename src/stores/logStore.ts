import { create } from 'zustand';

export type LogLevel = 'info' | 'warn' | 'error' | 'debug' | 'success';
export type LogSource =
  | 'sync'
  | 'ai'
  | 'search'
  | 'account'
  | 'system'
  | 'embeddings'
  | 'attachments'
  | 'chat'
  | 'memory'
  | 'tasks'
  | 'lens';

export interface LogEntry {
  id: number;
  timestamp: number;
  level: LogLevel;
  source: LogSource;
  message: string;
}

interface LogStore {
  entries: LogEntry[];
  isOpen: boolean;
  nextId: number;
  addLog: (level: LogLevel, source: LogSource, message: string) => void;
  clear: () => void;
  toggle: () => void;
  setOpen: (open: boolean) => void;
}

const MAX_ENTRIES = 500;

export const useLogStore = create<LogStore>((set) => ({
  entries: [],
  isOpen: false,
  nextId: 1,

  addLog: (level, source, message) =>
    set((state) => {
      const entry: LogEntry = {
        id: state.nextId,
        timestamp: Date.now(),
        level,
        source,
        message,
      };
      const entries = [...state.entries, entry];
      // Trim old entries if over limit
      if (entries.length > MAX_ENTRIES) {
        entries.splice(0, entries.length - MAX_ENTRIES);
      }
      return { entries, nextId: state.nextId + 1 };
    }),

  clear: () => set({ entries: [] }),

  toggle: () => set((state) => ({ isOpen: !state.isOpen })),

  setOpen: (open) => set({ isOpen: open }),
}));
