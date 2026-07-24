import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { type Toast, useToastStore } from '@/stores/toastStore';

const AUTO_DISMISS_MS = 8000;

/** Renders the toast stack bottom-right; each toast auto-dismisses. */
export function ToastHost() {
  const toasts = useToastStore((s) => s.toasts);
  if (toasts.length === 0) return null;
  return (
    <div className="fixed bottom-4 right-4 z-[60] flex flex-col gap-2 items-end">
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} />
      ))}
    </div>
  );
}

function ToastCard({ toast }: { toast: Toast }) {
  const { t } = useTranslation(['common']);
  const dismissToast = useToastStore((s) => s.dismissToast);

  useEffect(() => {
    // Sticky toasts stay until the user closes them (X or the action button).
    if (toast.sticky) return;
    const timer = setTimeout(() => dismissToast(toast.id), AUTO_DISMISS_MS);
    return () => clearTimeout(timer);
  }, [toast.id, toast.sticky, dismissToast]);

  return (
    <div className="flex items-center gap-3 pl-4 pr-2 py-2.5 bg-gray-900 text-white rounded-lg shadow-lg max-w-md">
      <span className="text-sm truncate">{toast.message}</span>
      {toast.actionLabel && (
        <button
          type="button"
          onClick={() => {
            toast.onAction?.();
            dismissToast(toast.id);
          }}
          className="flex-shrink-0 text-sm font-medium text-primary-300 hover:text-primary-200 transition-colors"
        >
          {toast.actionLabel}
        </button>
      )}
      <button
        type="button"
        onClick={() => dismissToast(toast.id)}
        className="flex-shrink-0 p-1 text-white/50 hover:text-white transition-colors"
        aria-label={t('common:actions.close')}
      >
        <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
