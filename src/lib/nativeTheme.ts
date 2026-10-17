// The native side of the theme: the window chrome (title bar on macOS and
// Windows, GTK decorations on Linux) follows the chosen theme, and with
// "system" it follows the OS again.
//
// Needs `core:window:allow-set-theme` in `src-tauri/capabilities/default.json`.
// Kept apart from `api.ts` because it is a window call, not an `invoke`.

import { getCurrentWindow } from '@tauri-apps/api/window';

import type { ThemePreference } from './theme';

export async function setNativeTheme(preference: ThemePreference): Promise<void> {
  await getCurrentWindow().setTheme(preference === 'system' ? null : preference);
}
