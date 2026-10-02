import { Fragment, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { SHORTCUT_GROUPS, SHORTCUTS, shortcutKeyLabels } from '@/lib/shortcuts';
import { useShortcutStore } from '@/stores/shortcutStore';
import { Modal } from './Modal';

/**
 * The `?` overlay: every shortcut, grouped, rendered from the registry so the
 * list cannot drift from what the keys actually do. Keys use the platform's
 * own modifier label (⌘ on macOS, Ctrl elsewhere).
 */
export function ShortcutHelpModal() {
  const { t } = useTranslation(['shortcuts']);
  const open = useShortcutStore((s) => s.helpOpen);
  const setHelpOpen = useShortcutStore((s) => s.setHelpOpen);
  const close = useCallback(() => setHelpOpen(false), [setHelpOpen]);
  const platform = useMemo(() => api.currentPlatform(), []);

  return (
    <Modal
      open={open}
      onClose={close}
      title={t('shortcuts:title')}
      subtitle={t('shortcuts:subtitle')}
      size="2xl"
      // Above Settings (z-50), which links here.
      zIndex={60}
    >
      <div data-testid="shortcut-help" className="grid gap-6 sm:grid-cols-2">
        {SHORTCUT_GROUPS.map((group) => (
          <section key={group} data-testid="shortcut-group">
            <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-gray-400">
              {t(`shortcuts:groups.${group}`)}
            </h3>
            <ul className="space-y-1.5">
              {SHORTCUTS.filter((s) => s.group === group).map((s) => (
                <li
                  key={s.id}
                  data-shortcut-id={s.id}
                  className="flex items-center justify-between gap-3 text-sm text-gray-200"
                >
                  <span className="min-w-0">{t(s.labelKey as 'shortcuts:items.next')}</span>
                  <span className="flex flex-shrink-0 items-center gap-1 text-xs text-gray-500">
                    {shortcutKeyLabels(s, platform).map((presses, alt) => (
                      <Fragment key={presses.join(' ')}>
                        {alt > 0 && <span>{t('shortcuts:or')}</span>}
                        {presses.map((label, i) => (
                          <Fragment key={label}>
                            {i > 0 && <span>{t('shortcuts:then')}</span>}
                            <kbd className="rounded border border-gray-600 bg-gray-800 px-1.5 py-0.5 font-mono text-[11px] text-gray-100">
                              {label}
                            </kbd>
                          </Fragment>
                        ))}
                      </Fragment>
                    ))}
                  </span>
                </li>
              ))}
            </ul>
          </section>
        ))}
      </div>
    </Modal>
  );
}
