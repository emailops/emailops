// Side drawer that shows the source email for a Lens row. Extracted values
// are not duplicated here — they already live in the table. Opened by
// clicking a row in `LensesView`.

import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';

import { EmailPreviewById } from '@/components/shared/EmailPreviewById';
import type { LensRow } from '@/types';

interface LensRowDrawerProps {
  row: LensRow | null;
  onClose: () => void;
}

export function LensRowDrawer({ row, onClose }: LensRowDrawerProps) {
  const { t } = useTranslation(['common', 'lenses']);
  // Esc closes the drawer.
  useEffect(() => {
    if (!row) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [row, onClose]);

  if (!row) return null;

  return (
    <div className="fixed inset-0 z-40 flex justify-end">
      {/* Backdrop — click to close. */}
      <button
        type="button"
        className="flex-1 bg-black/40 backdrop-blur-[1px]"
        onClick={onClose}
        aria-label={t('lenses:row.closeDrawer')}
      />

      {/* Panel */}
      <div className="flex h-full w-[min(46rem,80vw)] flex-col bg-[#1e1e1e] text-gray-200 shadow-2xl">
        <div className="flex items-center justify-between gap-3 border-b border-gray-700 px-5 py-3">
          <h2 className="truncate text-sm font-semibold text-gray-100">{t('lenses:row.sourceEmail')}</h2>
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-gray-600 px-2 py-0.5 text-xs text-gray-300 hover:bg-gray-700"
          >
            {t('common:actions.close')}
          </button>
        </div>

        <div className="min-h-0 flex-1 overflow-hidden">
          <EmailPreviewById accountId={row.accountId} emailId={row.emailId} emptyMessage="No email selected." />
        </div>
      </div>
    </div>
  );
}
