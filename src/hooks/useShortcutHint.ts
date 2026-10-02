import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { type ShortcutId, shortcutHint } from '@/lib/shortcuts';
import { useShortcutStore } from '@/stores/shortcutStore';

/** `hint(label, id)`: a toolbar tooltip naming the action's key ("Archive (E)"),
 *  or the bare label while keyboard shortcuts are off. */
export function useShortcutHint(): (label: string, id: ShortcutId) => string {
  const { t } = useTranslation(['shortcuts']);
  const enabled = useShortcutStore((s) => s.enabled);
  return useCallback(
    (label, id) =>
      shortcutHint(label, id, {
        enabled,
        platform: api.currentPlatform(),
        format: (l, keys) => t('shortcuts:withKeys', { label: l, keys }),
      }),
    [enabled, t],
  );
}
