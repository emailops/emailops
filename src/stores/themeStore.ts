// The resolved theme, published for components that need to *read* it.
//
// `useTheme` is the only writer and must stay the only caller of itself — it
// owns the class on <html>, and a second instance would fight it. Components
// that merely need to know whether the app is dark (the email frame, which
// renders into an iframe Tailwind cannot reach) read this instead.

import { create } from 'zustand';
import type { Theme } from '@/lib/theme';

interface ThemeStore {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

export const useThemeStore = create<ThemeStore>((set) => ({
  theme: 'light',
  setTheme: (theme) => set({ theme }),
}));
