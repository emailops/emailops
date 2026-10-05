import { type KeyboardEvent, useCallback } from 'react';
import * as api from '@/lib/api';
import { isSendShortcut } from '@/lib/shortcuts';
import { useShortcutStore } from '@/stores/shortcutStore';

/**
 * Cmd/Ctrl+Enter → the composer's one send function. Returns a handler for
 * the composer root's `onKeyDownCapture`: the capture phase runs before the
 * rich-text editor's own keymap, and marking the event handled makes
 * ProseMirror skip it, so the key sends instead of inserting a line break.
 */
export function useComposerSendKey(send: () => void, disabled: boolean) {
  return useCallback(
    (event: KeyboardEvent) => {
      // Cheap pre-check first: this runs on every key typed in the composer.
      if (event.key !== 'Enter' || !(event.metaKey || event.ctrlKey)) return;
      if (!useShortcutStore.getState().enabled) return;
      if (!isSendShortcut(event.nativeEvent, api.currentPlatform())) return;
      event.preventDefault();
      event.stopPropagation();
      if (!disabled) send();
    },
    [send, disabled],
  );
}
